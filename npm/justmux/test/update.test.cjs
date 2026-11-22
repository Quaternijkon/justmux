'use strict';

const assert = require('node:assert/strict');
const fs = require('node:fs');
const os = require('node:os');
const path = require('node:path');
const test = require('node:test');
const { EventEmitter } = require('node:events');
const { PassThrough } = require('node:stream');
const {
  maybeUpdate,
  isUpdateCommand,
  runUpdate,
  compareVersions,
  detectGlobalPrefix,
  updatePrompt,
} = require('../lib/update.cjs');

const DAY = 24 * 60 * 60 * 1000;
const NOW = 1_800_000_000_000;
const manifest = { name: 'justmux', version: '0.1.0' };

function fixture(t, overrides = {}) {
  const directory = fs.realpathSync(fs.mkdtempSync(path.join(os.tmpdir(), 'justmux-update-test-')));
  t.after(() => fs.rmSync(directory, { recursive: true, force: true }));
  const prefix = path.join(directory, 'npm prefix with spaces');
  const packageRoot = path.join(prefix, 'lib', 'node_modules', manifest.name);
  fs.mkdirSync(packageRoot, { recursive: true });
  const calls = { fetch: [], prompt: [], install: [], output: [] };
  const output = { isTTY: true, write: (text) => { calls.output.push(String(text)); } };
  const options = {
    manifest,
    packageRoot,
    cachePath: path.join(directory, 'state', 'update.json'),
    now: () => NOW,
    runtime: { stdin: { isTTY: true }, stdout: output, stderr: output,
      env: {}, platform: 'darwin', arch: 'arm64' },
    fetchImpl: async (...args) => {
      calls.fetch.push(args);
      return Response.json({ name: manifest.name, version: '0.2.0' });
    },
    prompt: async (...args) => { calls.prompt.push(args); return 'skip'; },
    install: async (...args) => { calls.install.push(args); return { status: 0, signal: null }; },
    ...overrides,
  };
  const cache = (value) => {
    fs.mkdirSync(path.dirname(options.cachePath), { recursive: true });
    fs.writeFileSync(options.cachePath, typeof value === 'string' ? value : JSON.stringify(value));
  };
  return { directory, prefix, packageRoot, options, calls, cache };
}

test('only interactive global npm startup checks for an update, including fullscreen and inline', async (t) => {
  for (const args of [[], ['--fullscreen'], ['--inline']]) {
    const app = fixture(t);
    assert.deepEqual(await maybeUpdate(args, app.options), { action: 'continue' });
    assert.equal(app.calls.fetch.length, 1);
    assert.equal(app.calls.prompt.length, 1);
    assert.deepEqual(app.calls.prompt[0][0], { currentVersion: '0.1.0', latestVersion: '0.2.0' });
    assert.equal(app.calls.install.length, 0);
  }
});

test('help, version, diagnostics and direct actions skip even in a TTY', async (t) => {
  for (const args of [['--help'], ['-h'], ['--version'], ['-V'], ['doctor'], ['resume'],
    ['resume', 'work'], ['--print-keybind'], ['--fullscreen', 'resume'], ['-L', 'private']]) {
    const app = fixture(t);
    assert.deepEqual(await maybeUpdate(args, app.options), { action: 'continue' });
    assert.equal(app.calls.fetch.length, 0, JSON.stringify(args));
    assert.equal(app.calls.prompt.length, 0);
    assert.equal(app.calls.install.length, 0);
  }
});

test('non-TTY, CI and explicit opt-out never contact the registry', async (t) => {
  for (const mode of ['stdin', 'stdout', 'CI', 'JUSTMUX_NO_UPDATE_CHECK', 'NO_UPDATE_CHECK']) {
    const app = fixture(t);
    if (mode === 'stdin' || mode === 'stdout') app.options.runtime[mode].isTTY = false;
    else app.options.runtime.env[mode] = '1';
    assert.deepEqual(await maybeUpdate([], app.options), { action: 'continue' });
    assert.equal(app.calls.fetch.length, 0, mode);
    assert.equal(app.calls.install.length, 0);
  }
});

test('local project and npx installations do not offer global updates', async (t) => {
  for (const suffix of ['project/node_modules/justmux', '.npm/_npx/123/node_modules/justmux']) {
    const app = fixture(t);
    app.options.packageRoot = path.join(app.directory, suffix);
    fs.mkdirSync(app.options.packageRoot, { recursive: true });
    assert.deepEqual(await maybeUpdate([], app.options), { action: 'continue' });
    assert.equal(app.calls.fetch.length, 0);
    assert.equal(app.calls.install.length, 0);
  }
});

test('fresh daily cache avoids requests and expires after one day', async (t) => {
  const app = fixture(t);
  app.cache({ version: manifest.version, time: NOW - DAY + 1 });
  assert.deepEqual(await maybeUpdate([], app.options), { action: 'continue' });
  assert.equal(app.calls.fetch.length, 0);
  app.options.now = () => NOW + 2;
  await maybeUpdate([], app.options);
  assert.equal(app.calls.fetch.length, 1);
});

test('corrupt, invalid or future-dated cache cannot permanently suppress checks', async (t) => {
  for (const value of ['{', { version: '0.2.0', time: NOW + DAY },
    { version: '0.2.0', time: 'today' }, { version: 'nonsense', time: NOW }, null]) {
    const app = fixture(t);
    app.cache(value);
    assert.deepEqual(await maybeUpdate([], app.options), { action: 'continue' });
    assert.equal(app.calls.fetch.length, 1, JSON.stringify(value));
    assert.equal(app.calls.install.length, 0);
  }
});

test('untrusted or malformed registry metadata is ignored without prompting', async (t) => {
  const bodies = ['{', 'null', JSON.stringify({ name: 'different-package', version: '0.2.0' }),
    JSON.stringify({ name: manifest.name, version: '0.2.0; touch /tmp/unexpected' }),
    JSON.stringify({ name: manifest.name, version: '99.0.0-beta.1' }),
    JSON.stringify({ name: manifest.name, version: '0.2.0', padding: 'x'.repeat(1024 * 1024) })];
  for (const body of bodies) {
    const app = fixture(t, { fetchImpl: async () => new Response(body) });
    assert.deepEqual(await maybeUpdate([], app.options), { action: 'continue' });
    assert.equal(app.calls.prompt.length, 0);
    assert.equal(app.calls.install.length, 0);
  }
});

test('network and HTTP failures quietly continue and avoid repeated startup requests', async (t) => {
  for (const fetchImpl of [async () => { throw new Error('offline'); },
    async () => new Response('unavailable', { status: 503 })]) {
    let requests = 0;
    const app = fixture(t, { fetchImpl: async (...args) => { requests++; return fetchImpl(...args); } });
    assert.deepEqual(await maybeUpdate([], app.options), { action: 'continue' });
    assert.deepEqual(await maybeUpdate([], app.options), { action: 'continue' });
    assert.equal(requests, 1);
    assert.equal(app.calls.prompt.length, 0);
    assert.equal(app.calls.install.length, 0);
    assert.deepEqual(app.calls.output, []);
  }
});

test('Skip only skips this launch while registry requests remain limited to once per day', async (t) => {
  const app = fixture(t);
  await maybeUpdate([], app.options);
  await maybeUpdate([], app.options);
  assert.equal(app.calls.fetch.length, 1);
  assert.equal(app.calls.prompt.length, 2);
  assert.equal(app.calls.install.length, 0);
  assert.equal(JSON.parse(fs.readFileSync(app.options.cachePath, 'utf8')).dismissedVersion, undefined);
  app.options.now = () => NOW + DAY + 1;
  await maybeUpdate([], app.options);
  assert.equal(app.calls.fetch.length, 2);
  assert.equal(app.calls.prompt.length, 3);
});

test('confirmation installs the exact version in the detected prefix with independent argv', async (t) => {
  const app = fixture(t, { prompt: async () => 'update' });
  const result = await maybeUpdate([], app.options);
  assert.deepEqual(result, { action: 'exit', code: 0 });
  assert.equal(app.calls.install.length, 1);
  const [executable, args, options] = app.calls.install[0];
  assert.equal(executable, 'npm');
  assert.ok(args.includes('install'));
  assert.ok(args.includes('--global') || args.includes('-g'));
  assert.ok(args.includes('justmux@0.2.0'));
  assert.equal(args[args.indexOf('--prefix') + 1], app.prefix);
  assert.equal(options.shell ?? false, false);
  assert.equal(options.stdio, 'inherit');
  assert.match(app.calls.output.join(''), /Run jm \(or justmux\) again/);
});

test('failed npm update keeps the existing native application available', async (t) => {
  const app = fixture(t, { prompt: async () => 'update', install: async () => ({ status: 1, signal: null }) });
  assert.deepEqual(await maybeUpdate([], app.options), { action: 'continue' });
  assert.match(app.calls.output.join(''), /failed|continue|existing|current/i);
  assert.equal(JSON.parse(fs.readFileSync(app.options.cachePath, 'utf8')).dismissedVersion, undefined);
  let retries = 0;
  app.options.prompt = async () => { retries++; return 'skip'; };
  await maybeUpdate([], app.options);
  assert.equal(retries, 1);
});

test('Skip this version persists across cache expiry but a later release is offered', async (t) => {
  let latest = '0.2.0';
  let requests = 0;
  const app = fixture(t, { prompt: async (...args) => { app.calls.prompt.push(args); return 'skip-version'; },
    fetchImpl: async () => { requests++; return Response.json({ name: manifest.name, version: latest }); } });
  await maybeUpdate([], app.options);
  assert.equal(JSON.parse(fs.readFileSync(app.options.cachePath, 'utf8')).dismissedVersion, latest);
  await maybeUpdate([], app.options);
  app.options.now = () => NOW + DAY + 1;
  await maybeUpdate([], app.options);
  assert.equal(requests, 2);
  assert.equal(app.calls.prompt.length, 1);
  latest = '0.3.0';
  app.options.now = () => NOW + 2 * DAY + 2;
  await maybeUpdate([], app.options);
  assert.equal(requests, 3);
  assert.equal(app.calls.prompt.length, 2);
  assert.deepEqual(app.calls.prompt[1][0], { currentVersion: '0.1.0', latestVersion: latest });
  assert.equal(app.calls.install.length, 0);
});

function promptFixture({ raw = false, flowing = null, columns = 80, rows = 24 } = {}) {
  const input = new PassThrough();
  input.isTTY = true;
  input.isRaw = raw;
  const rawChanges = [];
  input.setRawMode = (value) => { rawChanges.push(value); input.isRaw = value; return input; };
  if (flowing === true) input.resume();
  else if (flowing === false) input.pause();
  const output = new EventEmitter();
  output.isTTY = true;
  output.columns = columns;
  output.rows = rows;
  const writes = [];
  output.write = (value) => { writes.push(String(value)); return true; };
  const runtime = new EventEmitter();
  runtime.stdin = input;
  runtime.stdout = output;
  const existingSignal = () => {};
  runtime.on('SIGINT', existingSignal);
  const baseline = Object.fromEntries(['keypress', 'end', 'close', 'error'].map((name) => [name, input.listenerCount(name)]));
  const result = updatePrompt({ currentVersion: '0.1.0', latestVersion: '0.2.0' }, runtime);
  const key = (name, modifiers = {}) => input.emit('keypress', name.length === 1 ? name : '', { name, ...modifiers });
  const restored = () => {
    assert.equal(input.isRaw, raw);
    assert.equal(input.readableFlowing === true, flowing === true, 'originally inactive input must not keep Node alive');
    for (const [name, count] of Object.entries(baseline)) assert.equal(input.listenerCount(name), count, `leaked input ${name} listener`);
    for (const name of ['resize', 'error']) assert.equal(output.listenerCount(name), 0, `leaked output ${name} listener`);
    assert.deepEqual(runtime.listeners('SIGINT'), [existingSignal]);
    for (const name of ['SIGTERM', 'SIGHUP', 'SIGQUIT']) assert.equal(runtime.listenerCount(name), 0);
    const text = writes.join('');
    assert.ok(text.includes('\x1b[?25l'));
    assert.ok(text.endsWith('\x1b[?25h'), 'cursor must be visible after the picker');
    assert.doesNotMatch(text, /\x1b\[\?(?:1049|1000|1002|1003|1006)[hl]|\x1b\[(?:2|3)J/);
    input.destroy();
  };
  return { input, output, runtime, writes, rawChanges, result, key, restored };
}

test('inline picker defaults to Update and supports wrapping arrows, j/k and direct 1/2/3 choices', async () => {
  const cases = [
    [['return'], 'update'], [['down', 'return'], 'skip'], [['up', 'return'], 'skip-version'],
    [['j', 'j', 'j', 'return'], 'update'], [['k', 'k', 'return'], 'skip'],
    [['1'], 'update'], [['2'], 'skip'], [['3'], 'skip-version'], [['escape'], 'skip'],
  ];
  for (const [keys, expected] of cases) {
    const prompt = promptFixture();
    assert.match(prompt.writes.join(''), /\x1b\[7m> 1\. Update/);
    for (const key of keys) prompt.key(key);
    assert.equal(await prompt.result, expected);
    assert.deepEqual(prompt.rawChanges, [true, false]);
    prompt.restored();
  }
  // Node marks a standalone ESC as meta:true after its escape-sequence timer;
  // sending real input catches regressions hidden by synthetic key objects.
  const escape = promptFixture();
  escape.input.write('\x1b');
  assert.equal(await escape.result, 'skip');
  escape.restored();
});

test('inline picker restores raw and input state, ignores yes/no letters and handles EOF', async () => {
  for (const raw of [false, true]) {
    for (const flowing of [null, false, true]) {
      const prompt = promptFixture({ raw, flowing });
      let finished = false;
      prompt.result.then(() => { finished = true; });
      for (const key of ['y', 'e', 's', 'n', 'o']) prompt.key(key);
      await Promise.resolve();
      assert.equal(finished, false, 'yes/no letters must not confirm or skip');
      prompt.key('d', { ctrl: true });
      assert.equal(await prompt.result, 'skip');
      assert.deepEqual(prompt.rawChanges, raw ? [] : [true, false]);
      prompt.restored();
    }
  }
  const eof = promptFixture();
  eof.input.emit('end');
  assert.equal(await eof.result, 'skip');
  eof.restored();
});

test('Ctrl+C and termination signals restore the picker and bypass npm with their exit codes', async (t) => {
  const cases = [['SIGINT', 130], ['SIGTERM', 143], ['SIGHUP', 129], ['SIGQUIT', 131], ['key', 130]];
  for (const [signal, code] of cases) {
    const prompt = promptFixture();
    if (signal === 'key') prompt.key('c', { ctrl: true });
    else prompt.runtime.emit(signal);
    assert.deepEqual(await prompt.result, { action: 'exit', code });
    prompt.restored();
    const app = fixture(t, { prompt: async () => ({ action: 'exit', code }) });
    assert.deepEqual(await maybeUpdate([], app.options), { action: 'exit', code });
    assert.equal(app.calls.install.length, 0);
  }
});

test('resizing to a narrow short terminal keeps selection usable and removes resize listeners', async () => {
  const prompt = promptFixture();
  prompt.output.columns = 8;
  prompt.output.rows = 2;
  const start = prompt.writes.length;
  prompt.output.emit('resize');
  prompt.key('up');
  const rendered = prompt.writes.slice(start).join('').replace(/\x1b\[[0-9;?]*[A-Za-z]/g, '');
  for (const line of rendered.split(/[\r\n]/)) assert.ok(line.length <= 7, `wrapped line: ${line}`);
  prompt.key('return');
  assert.equal(await prompt.result, 'skip-version');
  prompt.restored();
});

test('bounded registry timeout aborts an unresponsive request and continues', async (t) => {
  let aborted = false;
  const app = fixture(t, {
    timeoutMs: 20,
    fetchImpl: async (_url, { signal }) => new Promise((_resolve, reject) => {
      signal.addEventListener('abort', () => { aborted = true; reject(new Error('aborted')); }, { once: true });
    }),
  });
  assert.deepEqual(await maybeUpdate([], app.options), { action: 'continue' });
  assert.equal(aborted, true);
  assert.equal(app.calls.prompt.length, 0);
  assert.equal(app.calls.install.length, 0);
});

test('default registry deadline permits a valid response taking longer than one second', async (t) => {
  const app = fixture(t, {
    fetchImpl: async (_url, { signal }) => new Promise((resolve, reject) => {
      const timer = setTimeout(() => {
        signal.removeEventListener('abort', abort);
        resolve(Response.json({ name: manifest.name, version: '0.2.0' }));
      }, 1100);
      const abort = () => { clearTimeout(timer); reject(new Error('request aborted before response')); };
      signal.addEventListener('abort', abort, { once: true });
    }),
  });
  // Use the production default rather than a test-specific timeout override.
  assert.deepEqual(await maybeUpdate([], app.options), { action: 'continue' });
  assert.equal(app.calls.prompt.length, 1, 'ordinary network latency must not suppress a valid update');
  assert.deepEqual(app.calls.prompt[0][0], { currentVersion: '0.1.0', latestVersion: '0.2.0' });
  assert.equal(app.calls.install.length, 0);
});

test('global prefix detection preserves scopes and resolves symlinks', (t) => {
  const app = fixture(t);
  assert.equal(detectGlobalPrefix(app.packageRoot, manifest.name), app.prefix);
  const scoped = path.join(app.prefix, 'lib', 'node_modules', '@owner', 'justmux');
  fs.mkdirSync(scoped, { recursive: true });
  assert.equal(detectGlobalPrefix(scoped, '@owner/justmux'), app.prefix);
  const alias = path.join(app.directory, 'package-alias');
  fs.symlinkSync(app.packageRoot, alias, 'dir');
  assert.equal(detectGlobalPrefix(alias, manifest.name), app.prefix);
  assert.equal(detectGlobalPrefix(app.packageRoot, 'different-package'), null);
});

test('numeric versions do not confuse 0.10 with 0.9 or suggest downgrades', async (t) => {
  assert.ok(compareVersions('0.10.0', '0.9.9') > 0);
  assert.ok(compareVersions('1.0.0', '1.0.0') === 0);
  assert.ok(compareVersions('0.1.0', '0.2.0') < 0);
  for (const version of ['0.1.0', '0.0.9']) {
    const app = fixture(t, { fetchImpl: async () => Response.json({ name: manifest.name, version }) });
    assert.deepEqual(await maybeUpdate([], app.options), { action: 'continue' });
    assert.equal(app.calls.prompt.length, 0);
    assert.equal(app.calls.install.length, 0);
  }
});

function explicitFixture(t, overrides = {}) {
  const app = fixture(t, overrides);
  const stdout = [];
  const stderr = [];
  const runtime = {
    stdin: { isTTY: false },
    stdout: { isTTY: false, write: (text) => { stdout.push(String(text)); } },
    stderr: { isTTY: false, write: (text) => { stderr.push(String(text)); } },
    env: { CI: '1', JUSTMUX_NO_UPDATE_CHECK: '1', NO_UPDATE_CHECK: '1' },
    platform: 'darwin',
    arch: 'arm64',
    ...overrides.runtime,
  };
  return {
    ...app,
    stdout,
    stderr,
    options: { ...app.options, ...overrides, runtime },
  };
}

test('isUpdateCommand recognizes update and upgrade subcommands only', () => {
  for (const args of [['update'], ['upgrade'], ['update', '--check'], ['upgrade', '--help']]) {
    assert.equal(isUpdateCommand(args), true, JSON.stringify(args));
  }
  for (const args of [[], ['doctor'], ['--fullscreen'], ['--inline'], ['resume'], null, undefined]) {
    assert.equal(isUpdateCommand(args), false, JSON.stringify(args));
  }
});

test('runUpdate upgrades global installs while bypassing passive opt-outs, cache, and dismissals', async (t) => {
  const app = explicitFixture(t);
  app.cache({ version: '0.1.0', dismissedVersion: '0.2.0', time: NOW });
  assert.deepEqual(await runUpdate([], app.options), { action: 'exit', code: 0 });
  assert.equal(app.calls.fetch.length, 1);
  assert.equal(app.calls.prompt.length, 0);
  assert.equal(app.calls.install.length, 1);
  assert.equal(app.calls.install[0][0], 'npm');
  assert.deepEqual(app.calls.install[0][1], [
    'install', '--global', '--prefix', app.prefix, 'justmux@0.2.0', '--include=optional',
  ]);
  const out = app.stdout.join('');
  assert.match(out, /Updating JustMux 0\.1\.0 -> 0\.2\.0\.\.\./);
  assert.match(out, /JustMux updated to 0\.2\.0\. Run jm \(or justmux\) again to start the new version\./);
  assert.equal(app.stderr.join(''), '');
  const saved = JSON.parse(fs.readFileSync(app.options.cachePath, 'utf8'));
  assert.equal(saved.version, '0.2.0');
  assert.equal(saved.dismissedVersion, undefined);
});

test('runUpdate reports up-to-date on same, older, or prerelease versions without installing', async (t) => {
  for (const version of ['0.1.0', '0.0.9', '0.2.0-beta.1']) {
    const app = explicitFixture(t, {
      fetchImpl: async () => Response.json({ name: manifest.name, version }),
    });
    app.cache({ version: '0.1.0', dismissedVersion: '0.2.0', time: NOW - 2 * DAY });
    assert.deepEqual(await runUpdate([], app.options), { action: 'exit', code: 0 });
    assert.equal(app.calls.install.length, 0);
    assert.match(app.stdout.join(''), /JustMux 0\.1\.0 is already up to date\./);
    assert.equal(app.stderr.join(''), '');
    if (!version.includes('-')) {
      const saved = JSON.parse(fs.readFileSync(app.options.cachePath, 'utf8'));
      assert.equal(saved.dismissedVersion, '0.2.0');
    }
  }
});

test('runUpdate --check reports availability without installing and refreshes cache', async (t) => {
  const app = explicitFixture(t);
  app.cache({ version: '0.1.0', dismissedVersion: '0.1.9', time: NOW - DAY });
  assert.deepEqual(await runUpdate(['--check'], app.options), { action: 'exit', code: 0 });
  assert.equal(app.calls.install.length, 0);
  const out = app.stdout.join('');
  assert.match(out, /JustMux update available: 0\.1\.0 -> 0\.2\.0/);
  assert.match(out, /Run 'jm update' \(or 'justmux update'\) to install\./);
  assert.equal(JSON.parse(fs.readFileSync(app.options.cachePath, 'utf8')).dismissedVersion, '0.1.9');
});

test('runUpdate -h/--help and unknown flags return expected exit codes without network', async (t) => {
  for (const flag of ['-h', '--help']) {
    const app = explicitFixture(t);
    assert.deepEqual(await runUpdate([flag], app.options), { action: 'exit', code: 0 });
    assert.match(app.stdout.join(''), /Usage: jm update \[--check\]/);
    assert.equal(app.calls.fetch?.length ?? 0, 0);
    assert.equal(app.calls.install.length, 0);
  }
  const bad = explicitFixture(t);
  assert.deepEqual(await runUpdate(['--bogus'], bad.options), { action: 'exit', code: 2 });
  assert.match(bad.stderr.join(''), /unexpected argument '--bogus' for 'update'/);
  assert.equal(bad.calls.install.length, 0);
});

test('runUpdate refuses self-update on non-global installs while allowing --check', async (t) => {
  const app = explicitFixture(t);
  app.options.packageRoot = path.join(app.directory, 'project', 'node_modules', 'justmux');
  fs.mkdirSync(app.options.packageRoot, { recursive: true });
  assert.deepEqual(await runUpdate([], app.options), { action: 'exit', code: 1 });
  assert.match(app.stderr.join(''), /cannot self-update a non-global installation/);
  assert.match(app.stderr.join(''), /npm install -g justmux@latest/);
  assert.equal(app.calls.install.length, 0);

  const checkApp = explicitFixture(t);
  checkApp.options.packageRoot = app.options.packageRoot;
  assert.deepEqual(await runUpdate(['--check'], checkApp.options), { action: 'exit', code: 0 });
  assert.match(checkApp.stdout.join(''), /JustMux update available: 0\.1\.0 -> 0\.2\.0/);
  assert.equal(checkApp.calls.install.length, 0);
});

test('runUpdate surfaces registry fetch and npm install failures cleanly', async (t) => {
  const netFail = explicitFixture(t, {
    fetchImpl: async () => { throw new Error('registry offline'); },
  });
  assert.deepEqual(await runUpdate([], netFail.options), { action: 'exit', code: 1 });
  assert.match(netFail.stderr.join(''), /failed to check npm for updates \(registry offline\)/);
  assert.match(netFail.stderr.join(''), /npm install -g justmux@latest/);
  assert.equal(netFail.calls.install.length, 0);

  const installFail = explicitFixture(t, {
    install: async () => ({ status: 7, signal: null }),
  });
  assert.deepEqual(await runUpdate([], installFail.options), { action: 'exit', code: 7 });
  assert.match(installFail.stderr.join(''), /JustMux update failed\./);
  assert.match(installFail.stderr.join(''), /To retry: npm 'install' '--global' '--prefix'/);
});
