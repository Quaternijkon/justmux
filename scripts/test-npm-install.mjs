#!/usr/bin/env node
// Exercise real npm packing, global command registration and updates entirely
// against an ephemeral loopback registry. Nothing is published or installed in
// the user's global prefix, and npm credentials/configuration are not used.
// --binary PATH selects a release artifact; --pty also exercises tmux handoff
// and two-client Popup behavior through the installed npm command.
import assert from 'node:assert/strict';
import { createHash } from 'node:crypto';
import { once } from 'node:events';
import fs from 'node:fs/promises';
import http from 'node:http';
import os from 'node:os';
import path from 'node:path';
import { spawn } from 'node:child_process';
import { fileURLToPath } from 'node:url';

const root = fileURLToPath(new URL('../', import.meta.url));
const temporary = await fs.mkdtemp(path.join(os.tmpdir(), 'justmux-npm-install-'));
const npm = process.platform === 'win32' ? 'npm.cmd' : 'npm';
const platform = `${process.platform}-${process.arch}`;
const runPty = process.argv.includes('--pty');
const requestedBinary = process.argv.indexOf('--binary');
const binary = requestedBinary === -1
  ? path.join(root, 'target', 'release', 'jm')
  : path.resolve(process.argv[requestedBinary + 1] ?? '');
const packages = new Map();
const tarballs = new Map();
const requests = [];
let registry;

const environment = Object.fromEntries(Object.entries(process.env).filter(([key]) => (
  !/^npm_config_/i.test(key) && !/^(NPM_TOKEN|NODE_AUTH_TOKEN)$/.test(key)
)));
environment.JUSTMUX_NO_UPDATE_CHECK = '1';

async function command(executable, args, {
  cwd = temporary, env = environment, allowFailure = false, timeout = 90_000,
} = {}) {
  return new Promise((resolve, reject) => {
    const child = spawn(executable, args, { cwd, env, stdio: ['ignore', 'pipe', 'pipe'] });
    let stdout = '';
    let stderr = '';
    const timer = setTimeout(() => {
      child.kill('SIGKILL');
      reject(new Error(`${path.basename(executable)} timed out: ${args.join(' ')}`));
    }, timeout);
    child.stdout.on('data', (chunk) => { stdout += chunk; });
    child.stderr.on('data', (chunk) => { stderr += chunk; });
    child.on('error', (error) => { clearTimeout(timer); reject(error); });
    child.on('close', (code, signal) => {
      clearTimeout(timer);
      if (!allowFailure && code !== 0) {
        reject(new Error(`${path.basename(executable)} ${args.join(' ')} failed (${code ?? signal})\n${stdout}\n${stderr}`));
      } else {
        resolve({ code, signal, stdout, stderr });
      }
    });
  });
}

async function pack(directory) {
  const manifest = JSON.parse(await fs.readFile(path.join(directory, 'package.json'), 'utf8'));
  const result = await command(npm, [
    'pack', directory, '--json', '--ignore-scripts', '--pack-destination', temporary,
    '--userconfig', path.join(temporary, 'empty.npmrc'),
    '--globalconfig', path.join(temporary, 'empty-global.npmrc'),
    '--cache', path.join(temporary, 'cache'),
  ]);
  const [{ filename }] = JSON.parse(result.stdout);
  const body = await fs.readFile(path.join(temporary, filename));
  tarballs.set(filename, body);
  if (!packages.has(manifest.name)) packages.set(manifest.name, { versions: {}, latest: manifest.version });
  packages.get(manifest.name).versions[manifest.version] = {
    ...manifest,
    dist: {
      tarball: `${registry}/tarballs/${filename}`,
      integrity: `sha512-${createHash('sha512').update(body).digest('base64')}`,
      shasum: createHash('sha1').update(body).digest('hex'),
    },
  };
  return manifest;
}

async function install(prefix, packageName, extra = [], { scripts = false, env = environment } = {}) {
  const args = [
    'install', '--global', '--prefix', prefix, `${packageName}@latest`,
    '--registry', registry, '--userconfig', path.join(temporary, 'empty.npmrc'),
    '--globalconfig', path.join(temporary, 'empty-global.npmrc'),
    '--cache', path.join(temporary, 'cache'),
  ];
  if (!scripts) args.push('--ignore-scripts');
  args.push('--no-audit', '--no-fund', '--fetch-retries=0', '--fetch-timeout=15000', '--prefer-online', ...extra);
  return command(npm, args, { env });
}

async function invoke(prefix, name, args, options = {}) {
  // Bare names exercise npm's actual bin symlinks and PATH lookup.
  return command(name, args, {
    ...options,
    env: { ...environment, PATH: `${path.join(prefix, 'bin')}${path.delimiter}${process.env.PATH ?? ''}` },
  });
}

async function updateFixture(mainSource, version) {
  const directory = path.join(temporary, `fixture-${version}`);
  const mainDirectory = path.join(directory, 'main');
  await fs.cp(mainSource, mainDirectory, { recursive: true });
  const manifest = JSON.parse(await fs.readFile(path.join(mainDirectory, 'package.json'), 'utf8'));
  manifest.version = version;
  delete manifest.private;
  manifest.optionalDependencies = Object.fromEntries(
    Object.keys(manifest.optionalDependencies).map((name) => [name, version]),
  );
  await fs.writeFile(path.join(mainDirectory, 'package.json'), `${JSON.stringify(manifest, null, 2)}\n`);
  const nativeDirectory = path.join(directory, 'native');
  await fs.mkdir(path.join(nativeDirectory, 'bin'), { recursive: true });
  await fs.writeFile(path.join(nativeDirectory, 'package.json'), `${JSON.stringify({
    name: `${manifest.name}-${platform}`, version, license: 'MIT',
    os: [process.platform], cpu: [process.arch], files: ['bin/'],
  }, null, 2)}\n`);
  // Distinct, explicit test fixtures exercise npm replacing the actual optional
  // native dependency. Production tarballs above always use the real Rust binary.
  await fs.writeFile(path.join(nativeDirectory, 'bin', 'jm'),
    `#!/bin/sh\nprintf '%s\\n' 'native-fixture ${version}'\nprintf '<%s>\\n' "$@"\n`, { mode: 0o755 });
  await pack(nativeDirectory);
  await pack(mainDirectory);
}

const server = http.createServer((request, response) => {
  requests.push(request.url);
  if (request.method !== 'GET') {
    response.writeHead(405).end('This test registry is read-only');
    return;
  }
  const route = decodeURIComponent(new URL(request.url, registry).pathname).replace(/^\//, '');
  if (route.startsWith('tarballs/')) {
    const body = tarballs.get(route.slice('tarballs/'.length));
    response.writeHead(body ? 200 : 404, { 'Content-Type': 'application/octet-stream' }).end(body);
    return;
  }
  const entry = packages.get(route);
  response.writeHead(entry ? 200 : 404, { 'Content-Type': 'application/json' }).end(JSON.stringify(
    entry ? { name: route, 'dist-tags': { latest: entry.latest }, versions: entry.versions }
      : { error: 'not_found', reason: 'Not a package in this isolated fixture registry' },
  ));
});

try {
  assert.ok(typeof process.execve === 'function', 'Use Node.js >=22.15 with process.execve support');
  await fs.access(binary);
  await fs.writeFile(path.join(temporary, 'empty.npmrc'), '');
  await fs.writeFile(path.join(temporary, 'empty-global.npmrc'), '');
  server.listen(0, '127.0.0.1');
  await once(server, 'listening');
  registry = `http://127.0.0.1:${server.address().port}`;

  const generated = path.join(temporary, 'actual');
  for (const options of [
    ['--main'],
    ['--platform', platform, '--binary', binary],
  ]) {
    await command(process.execPath, [path.join(root, 'scripts', 'npm-packages.mjs'),
      'prepare', ...options, '--out', generated]);
  }
  const nativeDirectory = path.join(generated, `justmux-${platform}`);
  const mainDirectory = path.join(generated, 'justmux');
  await pack(nativeDirectory);
  const manifest = await pack(mainDirectory);
  assert.notEqual(manifest.private, true, 'prepared release must be publishable');
  for (const lifecycle of ['preinstall', 'install']) {
    assert.equal(manifest.scripts?.[lifecycle], undefined,
      `installation must not require a ${lifecycle} download/build script`);
  }
  assert.equal(manifest.scripts?.postinstall, 'node lib/postinstall.cjs',
    'prepared release includes the local PATH/bin-link postinstall helper');
  const prefix = path.join(temporary, 'actual-prefix');
  await install(prefix, manifest.name);
  const expected = await command(binary, ['--version']);
  for (const name of ['jm', 'justmux']) {
    const version = await invoke(prefix, name, ['--version']);
    assert.equal(version.stdout.trim(), expected.stdout.trim(), `${name} reports the real native binary version`);
    const help = await invoke(prefix, name, ['--help']);
    assert.match(help.stdout, /--fullscreen/);
    assert.match(help.stdout, /--socket-name/);
  }
  console.log('PASS actual npm install: packed Rust binary, jm + justmux through PATH, --version and --help');

  const toolBin = path.join(temporary, 'tool-bin');
  await fs.mkdir(toolBin, { recursive: true });
  await fs.symlink(process.execPath, path.join(toolBin, 'node'));
  const resolvedNpm = (await command('which', [npm])).stdout.trim();

  // Hidden prefix not in PATH: postinstall links jm & justmux into $HOME/.local/bin.
  const tempHome = path.join(temporary, 'hidden-home');
  const localBin = path.join(tempHome, '.local', 'bin');
  const hiddenPrefix = path.join(temporary, 'hidden-prefix');
  await fs.mkdir(localBin, { recursive: true });
  await fs.mkdir(hiddenPrefix, { recursive: true });
  const hiddenEnv = {
    ...environment,
    HOME: tempHome,
    PATH: `${localBin}${path.delimiter}${toolBin}${path.delimiter}/usr/bin${path.delimiter}/bin`,
  };
  await command(resolvedNpm, [
    'install', '--global', '--prefix', hiddenPrefix, `${manifest.name}@latest`,
    '--registry', registry, '--userconfig', path.join(temporary, 'empty.npmrc'),
    '--globalconfig', path.join(temporary, 'empty-global.npmrc'),
    '--cache', path.join(temporary, 'cache'), '--no-audit', '--no-fund',
    '--fetch-retries=0', '--fetch-timeout=15000', '--prefer-online',
  ], { env: hiddenEnv });
  for (const name of ['jm', 'justmux']) {
    const linkPath = path.join(localBin, name);
    assert.equal((await fs.lstat(linkPath)).isSymbolicLink(), true, `${name} symlink must be created in $HOME/.local/bin`);
    const out = await command(name, ['--version'], { env: hiddenEnv });
    assert.equal(out.stdout.trim(), expected.stdout.trim(), `hidden-prefix ${name} executes directly from PATH`);
  }
  console.log('PASS hidden-prefix PATH recovery: postinstall links jm + justmux into $HOME/.local/bin');

  // --bin-links=false recovery: postinstall restores <prefix>/bin symlinks.
  const noBinLinksPrefix = path.join(temporary, 'no-bin-links-prefix');
  const noBinLinksBin = path.join(noBinLinksPrefix, 'bin');
  const noBinLinksHome = path.join(temporary, 'no-bin-links-home');
  await fs.mkdir(noBinLinksBin, { recursive: true });
  await fs.mkdir(noBinLinksHome, { recursive: true });
  const noBinLinksEnv = {
    ...environment,
    HOME: noBinLinksHome,
    PATH: `${noBinLinksBin}${path.delimiter}${toolBin}${path.delimiter}/usr/bin${path.delimiter}/bin`,
  };
  await command(resolvedNpm, [
    'install', '--global', '--prefix', noBinLinksPrefix, `${manifest.name}@latest`,
    '--bin-links=false', '--registry', registry, '--userconfig', path.join(temporary, 'empty.npmrc'),
    '--globalconfig', path.join(temporary, 'empty-global.npmrc'),
    '--cache', path.join(temporary, 'cache'), '--no-audit', '--no-fund',
    '--fetch-retries=0', '--fetch-timeout=15000', '--prefer-online',
  ], { env: noBinLinksEnv });
  for (const name of ['jm', 'justmux']) {
    const linkPath = path.join(noBinLinksBin, name);
    assert.equal((await fs.lstat(linkPath)).isSymbolicLink(), true, `${name} symlink must be restored in <prefix>/bin`);
    const out = await command(name, ['--version'], { env: noBinLinksEnv });
    assert.equal(out.stdout.trim(), expected.stdout.trim(), `bin-links=false ${name} executes from <prefix>/bin`);
  }
  await assert.rejects(fs.lstat(path.join(noBinLinksHome, '.local', 'bin')), { code: 'ENOENT' });
  console.log('PASS --bin-links=false recovery: postinstall restores <prefix>/bin symlinks');

  if (runPty) {
    const prompt = await command('python3', [path.join(root, 'scripts/test-npm-update-pty.py'),
      '--entry', path.join(prefix, 'bin', 'jm')], { timeout: 60_000 });
    process.stdout.write(prompt.stdout);
    const pty = await command('python3', ['-c', `
import importlib.util
import sys
from pathlib import Path

spec = importlib.util.spec_from_file_location('justmux_installed_pty', sys.argv[1])
smoke = importlib.util.module_from_spec(spec)
spec.loader.exec_module(smoke)
smoke.JM = Path(sys.argv[2])
for scenario in (smoke.external, smoke.popup):
    socket = smoke.Socket()
    try:
        scenario(socket)
    finally:
        socket.cleanup()
`, path.join(root, 'scripts', 'pty_smoke.py'), path.join(prefix, 'bin', 'jm')], { timeout: 60_000 });
    process.stdout.write(pty.stdout);
    console.log('PASS installed npm launcher PTY: exec handoff to native tmux and two-client Popup switch');
  }

  const incomplete = path.join(temporary, 'missing-native-prefix');
  await install(incomplete, manifest.name, ['--omit=optional']);
  // Some npm versions retain optional packages even with a global --omit flag.
  // Remove only this disposable install's native dependency so the launcher
  // error is tested deterministically across those npm implementations.
  const incompleteMain = path.join(incomplete, 'lib', 'node_modules', ...manifest.name.split('/'));
  await fs.rm(path.join(incompleteMain, 'node_modules', ...`${manifest.name}-${platform}`.split('/')),
    { recursive: true, force: true });
  const missing = await invoke(incomplete, 'jm', ['--version'], { allowFailure: true });
  assert.notEqual(missing.code, 0, 'an incomplete install must not silently use another version');
  assert.match(missing.stderr, /native package.*missing/i);
  assert.match(missing.stderr, /--include=optional/);
  console.log('PASS missing optional dependency: actionable reinstall message instead of a crash');

  const updatePrefix = path.join(temporary, 'update-prefix');
  await updateFixture(mainDirectory, '1.0.0');
  packages.get(manifest.name).latest = '1.0.0';
  await install(updatePrefix, manifest.name);
  assert.match((await invoke(updatePrefix, 'jm', ['--version'])).stdout, /native-fixture 1\.0\.0/);
  await updateFixture(mainDirectory, '1.0.1');
  packages.get(manifest.name).latest = '1.0.1';
  await install(updatePrefix, manifest.name);
  const arguments_ = ['space here', '中文', '; echo NOT_EXECUTED', "quote'\"", '$HOME', '--flag'];
  for (const name of ['jm', 'justmux']) {
    const updated = await invoke(updatePrefix, name, arguments_);
    assert.equal(updated.stdout, `native-fixture 1.0.1\n${arguments_.map((argument) => `<${argument}>\n`).join('')}`);
  }
  console.log('PASS npm update: both commands move from fixture 1.0.0 to 1.0.1, arguments preserved without shell execution');
  assert.ok(requests.some((url) => url.startsWith('/tarballs/')), 'npm must download packed artifacts from the fixture registry');
  console.log('All npm install checks passed against an isolated loopback registry; no real registry publication or global changes.');
} finally {
  server.closeAllConnections();
  if (server.listening) await new Promise((resolve) => server.close(resolve));
  await fs.rm(temporary, { recursive: true, force: true });
}
