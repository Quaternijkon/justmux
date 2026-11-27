import assert from 'node:assert/strict';
import { createHash } from 'node:crypto';
import { execFileSync, spawn } from 'node:child_process';
import { once } from 'node:events';
import fs from 'node:fs/promises';
import http from 'node:http';
import os from 'node:os';
import path from 'node:path';
import test from 'node:test';
import { fileURLToPath } from 'node:url';

const ROOT = fileURLToPath(new URL('../', import.meta.url));
const SCRIPT = path.join(ROOT, 'install.sh');

function platformPackageName() {
  return `justmux-${process.platform}-${process.arch}`;
}

async function createMockReleaseServer(t, { version = '9.9.9', corruptMetadataSha = false } = {}) {
  const work = await fs.mkdtemp(path.join(os.tmpdir(), 'justmux-install-sh-srv-'));
  t.after(() => fs.rm(work, { recursive: true, force: true }));

  const pkgRoot = path.join(work, 'package');
  await fs.mkdir(path.join(pkgRoot, 'bin'), { recursive: true });
  const binaryContent = `#!/bin/sh\necho "jm ${version}"\n`;
  await fs.writeFile(path.join(pkgRoot, 'bin', 'jm'), binaryContent, { mode: 0o755 });
  const actualSha = createHash('sha256').update(binaryContent).digest('hex');

  const pkgJson = {
    name: platformPackageName(),
    version,
    justmux: {
      target: process.platform === 'darwin' ? 'aarch64-apple-darwin' : 'x86_64-unknown-linux-musl',
      sha256: actualSha,
    },
  };
  await fs.writeFile(path.join(pkgRoot, 'package.json'), `${JSON.stringify(pkgJson, null, 2)}\n`);

  const tarballPath = path.join(work, `${pkgJson.name}-${version}.tgz`);
  execFileSync('tar', ['-czf', tarballPath, '-C', work, 'package']);
  const tarballBytes = await fs.readFile(tarballPath);

  let registryUrl = '';
  const server = http.createServer((req, res) => {
    const pathname = new URL(req.url, registryUrl).pathname;
    if (pathname === '/-/package/justmux/dist-tags') {
      res.writeHead(200, { 'Content-Type': 'application/json' }).end(JSON.stringify({ latest: version }));
      return;
    }
    if (pathname === `/${pkgJson.name}/${version}`) {
      const advertisedSha = corruptMetadataSha ? '0'.repeat(64) : actualSha;
      res.writeHead(200, { 'Content-Type': 'application/json' }).end(JSON.stringify({
        name: pkgJson.name,
        version,
        dist: {
          tarball: `${registryUrl}/${pkgJson.name}/-/${pkgJson.name}-${version}.tgz`,
        },
        justmux: {
          target: pkgJson.justmux.target,
          sha256: advertisedSha,
        },
      }));
      return;
    }
    if (pathname === `/${pkgJson.name}/-/${pkgJson.name}-${version}.tgz`) {
      res.writeHead(200, { 'Content-Type': 'application/octet-stream' }).end(tarballBytes);
      return;
    }
    res.writeHead(404).end('not found');
  });

  server.listen(0, '127.0.0.1');
  await once(server, 'listening');
  registryUrl = `http://127.0.0.1:${server.address().port}`;
  t.after(() => {
    server.closeAllConnections();
    server.close();
  });

  return { registryUrl, actualSha, version };
}

function runShell(args, { env }) {
  return new Promise((resolve, reject) => {
    const child = spawn('/bin/sh', args, { env, stdio: ['ignore', 'pipe', 'pipe'] });
    let stdout = '';
    let stderr = '';
    child.stdout.on('data', (chunk) => { stdout += chunk; });
    child.stderr.on('data', (chunk) => { stderr += chunk; });
    child.on('error', reject);
    child.on('close', (code) => resolve({ code, stdout, stderr }));
  });
}

function findCommand(name) {
  try {
    return execFileSync('/bin/sh', ['-c', `command -v ${name}`], { encoding: 'utf8' }).trim();
  } catch {
    return null;
  }
}

async function buildToolDir(dir, names) {
  await fs.mkdir(dir, { recursive: true });
  for (const name of names) {
    const found = findCommand(name);
    if (found) {
      await fs.symlink(found, path.join(dir, name)).catch(() => {});
    }
  }
}

test('install.sh passes POSIX sh syntax check', async () => {
  const res = await runShell(['-n', SCRIPT], { env: { PATH: process.env.PATH } });
  assert.equal(res.code, 0, res.stderr);
});

test('explicit JUSTMUX_INSTALL_DIR installs jm and justmux symlink', async (t) => {
  const srv = await createMockReleaseServer(t);
  const tmp = await fs.mkdtemp(path.join(os.tmpdir(), 'justmux-inst-explicit-'));
  t.after(() => fs.rm(tmp, { recursive: true, force: true }));

  const installDir = path.join(tmp, 'custom-bin');
  const homeDir = path.join(tmp, 'home');
  await fs.mkdir(homeDir, { recursive: true });

  const res = await runShell([SCRIPT], {
    env: {
      PATH: process.env.PATH,
      HOME: homeDir,
      SHELL: '/bin/bash',
      JUSTMUX_REGISTRY: srv.registryUrl,
      JUSTMUX_INSTALL_DIR: installDir,
      JUSTMUX_NO_MODIFY_PATH: '1',
    },
  });
  assert.equal(res.code, 0, res.stderr);

  const jmStat = await fs.lstat(path.join(installDir, 'jm'));
  const justmuxStat = await fs.lstat(path.join(installDir, 'justmux'));
  assert.equal(jmStat.isFile(), true);
  assert.equal(justmuxStat.isSymbolicLink(), true);
  assert.equal(await fs.readlink(path.join(installDir, 'justmux')), 'jm');

  for (const cmd of ['jm', 'justmux']) {
    const out = execFileSync(path.join(installDir, cmd), ['--version'], { encoding: 'utf8' }).trim();
    assert.equal(out, `jm ${srv.version}`);
  }
});

test('auto-detects writable $HOME/.local/bin already on PATH without editing ~/.bashrc', async (t) => {
  const srv = await createMockReleaseServer(t);
  const tmp = await fs.mkdtemp(path.join(os.tmpdir(), 'justmux-inst-autopath-'));
  t.after(() => fs.rm(tmp, { recursive: true, force: true }));

  const homeDir = path.join(tmp, 'home');
  const localBin = path.join(homeDir, '.local', 'bin');
  const tools = path.join(tmp, 'tools');
  await fs.mkdir(localBin, { recursive: true });
  await buildToolDir(tools, [
    'uname', 'curl', 'wget', 'tar', 'gzip', 'awk', 'sed', 'grep', 'mktemp',
    'rm', 'cp', 'chmod', 'mv', 'ln', 'mkdir', 'dirname', 'basename',
    'sha256sum', 'shasum', 'openssl', 'python3', 'node',
  ]);

  const controlledPath = `${localBin}${path.delimiter}${tools}`;
  const res = await runShell([SCRIPT], {
    env: {
      PATH: controlledPath,
      HOME: homeDir,
      SHELL: '/bin/bash',
      JUSTMUX_REGISTRY: srv.registryUrl,
      JUSTMUX_SKIP_SYSTEM_DIRS: '1',
    },
  });
  assert.equal(res.code, 0, res.stderr);

  await assert.rejects(fs.stat(path.join(homeDir, '.bashrc')), { code: 'ENOENT' });
  for (const cmd of ['jm', 'justmux']) {
    const out = execFileSync(cmd, ['--version'], {
      env: { PATH: controlledPath },
      encoding: 'utf8',
    }).trim();
    assert.equal(out, `jm ${srv.version}`);
  }
});

test('falls back to $HOME/.local/bin when not in PATH, updates ~/.bashrc idempotently, and respects JUSTMUX_NO_MODIFY_PATH', async (t) => {
  const srv = await createMockReleaseServer(t);
  const tmp = await fs.mkdtemp(path.join(os.tmpdir(), 'justmux-inst-rc-'));
  t.after(() => fs.rm(tmp, { recursive: true, force: true }));

  const homeDir = path.join(tmp, 'home');
  const tools = path.join(tmp, 'tools');
  await fs.mkdir(homeDir, { recursive: true });
  await buildToolDir(tools, [
    'uname', 'curl', 'wget', 'tar', 'gzip', 'awk', 'sed', 'grep', 'mktemp',
    'rm', 'cp', 'chmod', 'mv', 'ln', 'mkdir', 'dirname', 'basename',
    'sha256sum', 'shasum', 'openssl', 'python3', 'node',
  ]);

  const baseEnv = {
    PATH: tools,
    HOME: homeDir,
    SHELL: '/bin/bash',
    JUSTMUX_REGISTRY: srv.registryUrl,
    JUSTMUX_SKIP_SYSTEM_DIRS: '1',
  };

  // With JUSTMUX_NO_MODIFY_PATH=1: installs without creating ~/.bashrc.
  const noRc = await runShell([SCRIPT], { env: { ...baseEnv, JUSTMUX_NO_MODIFY_PATH: '1' } });
  assert.equal(noRc.code, 0, noRc.stderr);
  await assert.rejects(fs.stat(path.join(homeDir, '.bashrc')), { code: 'ENOENT' });
  const localBin = path.join(homeDir, '.local', 'bin');
  assert.equal(execFileSync(path.join(localBin, 'jm'), ['--version'], { encoding: 'utf8' }).trim(), `jm ${srv.version}`);

  // Run twice without JUSTMUX_NO_MODIFY_PATH: appends PATH export once.
  const first = await runShell([SCRIPT], { env: baseEnv });
  assert.equal(first.code, 0, first.stderr);
  const second = await runShell([SCRIPT], { env: baseEnv });
  assert.equal(second.code, 0, second.stderr);

  const bashrc = await fs.readFile(path.join(homeDir, '.bashrc'), 'utf8');
  const matches = bashrc.match(new RegExp(localBin.replace(/[.*+?^${}()|[\]\\]/g, '\\$&'), 'g')) ?? [];
  assert.equal(matches.length, 1, `expected PATH entry once in ~/.bashrc, got:\n${bashrc}`);
});

test('SHA-256 mismatch aborts without overwriting existing installed binary', async (t) => {
  const srv = await createMockReleaseServer(t, { corruptMetadataSha: true });
  const tmp = await fs.mkdtemp(path.join(os.tmpdir(), 'justmux-inst-badsha-'));
  t.after(() => fs.rm(tmp, { recursive: true, force: true }));

  const installDir = path.join(tmp, 'bin');
  await fs.mkdir(installDir, { recursive: true });
  const existingJm = path.join(installDir, 'jm');
  await fs.writeFile(existingJm, '#!/bin/sh\necho existing-binary\n', { mode: 0o755 });

  const res = await runShell([SCRIPT], {
    env: {
      PATH: process.env.PATH,
      HOME: tmp,
      JUSTMUX_REGISTRY: srv.registryUrl,
      JUSTMUX_INSTALL_DIR: installDir,
      JUSTMUX_NO_MODIFY_PATH: '1',
    },
  });
  assert.notEqual(res.code, 0);
  assert.match(res.stderr, /SHA-256|mismatch/i);
  assert.equal(await fs.readFile(existingJm, 'utf8'), '#!/bin/sh\necho existing-binary\n');
});

test('pure POSIX JSON parser fallback works without python3 or node on PATH', async (t) => {
  const srv = await createMockReleaseServer(t, { version: '8.7.6' });
  const tmp = await fs.mkdtemp(path.join(os.tmpdir(), 'justmux-inst-posix-'));
  t.after(() => fs.rm(tmp, { recursive: true, force: true }));

  const homeDir = path.join(tmp, 'home');
  const localBin = path.join(homeDir, '.local', 'bin');
  const posixTools = path.join(tmp, 'posix-tools');
  await fs.mkdir(localBin, { recursive: true });
  await buildToolDir(posixTools, [
    'uname', 'curl', 'wget', 'tar', 'gzip', 'awk', 'sed', 'grep', 'mktemp',
    'rm', 'cp', 'chmod', 'mv', 'ln', 'mkdir', 'dirname', 'basename',
    'sha256sum', 'shasum', 'openssl',
  ]);

  const controlledPath = `${localBin}${path.delimiter}${posixTools}`;
  const res = await runShell([SCRIPT], {
    env: {
      PATH: controlledPath,
      HOME: homeDir,
      SHELL: '/bin/sh',
      JUSTMUX_REGISTRY: srv.registryUrl,
      JUSTMUX_SKIP_SYSTEM_DIRS: '1',
    },
  });
  assert.equal(res.code, 0, res.stderr);
  assert.equal(execFileSync(path.join(localBin, 'jm'), ['--version'], { encoding: 'utf8' }).trim(), 'jm 8.7.6');
  assert.equal(execFileSync(path.join(localBin, 'justmux'), ['--version'], { encoding: 'utf8' }).trim(), 'jm 8.7.6');
});
