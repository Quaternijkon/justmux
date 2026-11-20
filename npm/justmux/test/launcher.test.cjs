'use strict';

const assert = require('node:assert/strict');
const test = require('node:test');
const fs = require('node:fs');
const os = require('node:os');
const path = require('node:path');
const { spawnSync } = require('node:child_process');
const { SUPPORTED_PLATFORMS, platformPackageName, resolveNativeExecutable, run } = require('../lib/launcher.cjs');

const manifest = { name: '@owner/justmux', version: '0.1.0' };

test('platform names preserve npm scopes and cover the release targets', () => {
  assert.deepEqual(SUPPORTED_PLATFORMS.map((entry) => entry.target), [
    'aarch64-apple-darwin', 'x86_64-unknown-linux-musl',
  ]);
  assert.equal(platformPackageName('justmux', 'darwin', 'arm64'), 'justmux-darwin-arm64');
  assert.equal(platformPackageName('@owner/justmux', 'linux', 'x64'), '@owner/justmux-linux-x64');
  assert.throws(() => platformPackageName('justmux', 'win32', 'x64'), /Unsupported platform win32\/x64/);
  assert.throws(() => platformPackageName('justmux', 'linux', 'riscv64'), /Unsupported platform/);
  assert.throws(() => platformPackageName('justmux', 'darwin', 'x64'), /Unsupported platform/);
  assert.throws(() => platformPackageName('justmux', 'linux', 'arm64'), /Unsupported platform/);
});

test('both installed command names use the same launcher and exact dependencies', () => {
  const pkg = require('../package.json');
  assert.equal(pkg.bin.jm, pkg.bin.justmux);
  assert.equal(Object.keys(pkg.optionalDependencies).length, SUPPORTED_PLATFORMS.length);
  for (const { platform, arch } of SUPPORTED_PLATFORMS) {
    assert.equal(pkg.optionalDependencies[platformPackageName(pkg.name, platform, arch)], pkg.version);
  }
  assert.equal(pkg.scripts.postinstall, undefined);
});

test('native resolution validates the installed version and executable', (t) => {
  const directory = fs.mkdtempSync(path.join(os.tmpdir(), 'justmux-native-'));
  t.after(() => fs.rmSync(directory, { recursive: true, force: true }));
  const packageFile = path.join(directory, 'package.json');
  const executable = path.join(directory, 'bin', 'jm');
  fs.mkdirSync(path.dirname(executable));
  fs.writeFileSync(packageFile, JSON.stringify({ name: '@owner/justmux-darwin-arm64', version: '0.1.0' }));
  fs.writeFileSync(executable, '#!/bin/sh\nexit 0\n', { mode: 0o755 });
  const options = { platform: 'darwin', arch: 'arm64', resolvePackage: () => packageFile };
  assert.equal(resolveNativeExecutable(manifest, options), executable);
  fs.writeFileSync(packageFile, JSON.stringify({ name: '@owner/justmux-darwin-arm64', version: '0.0.9' }));
  assert.throws(() => resolveNativeExecutable(manifest, options), /must be .*@0\.1\.0; found .*@0\.0\.9/);
});

test('missing optional dependency gives an actionable exact-version reinstall command', () => {
  assert.throws(() => resolveNativeExecutable(manifest, {
    platform: 'linux', arch: 'x64', resolvePackage() { throw new Error('not installed'); },
  }), /npm install -g @owner\/justmux@0\.1\.0 --include=optional/);
});

test('missing executable and malformed metadata report useful errors', () => {
  const options = {
    platform: 'linux', arch: 'x64', resolvePackage: () => '/unused/package.json',
    readFile: () => JSON.stringify({ name: '@owner/justmux-linux-x64', version: '0.1.0' }),
    access() { throw new Error('missing'); },
  };
  assert.throws(() => resolveNativeExecutable(manifest, options), /missing or not executable/);
  assert.throws(() => resolveNativeExecutable(manifest, { ...options, readFile: () => '{' }), /Cannot read/);
});

test('exec replacement passes argv and environment unchanged without invoking a shell', () => {
  let invoked;
  const args = ['new', 'my project', "$(touch /tmp/should-not-exist); '中文'"];
  const env = { TERM: 'xterm-256color', TMUX: '/private/server,1,0' };
  run(args, {
    manifest,
    runtime: { platform: 'linux', arch: 'x64', env, execve(...values) { invoked = values; } },
    resolveExecutable: () => '/native/with spaces/jm',
  });
  assert.deepEqual(invoked, ['/native/with spaces/jm', ['/native/with spaces/jm', ...args], env]);
});

test('older runtimes fail instead of leaving a supervising Node process', () => {
  assert.throws(() => run([], {
    manifest, runtime: { platform: 'darwin', arch: 'arm64' },
    resolveExecutable() { assert.fail('must not resolve a binary on an unsupported runtime'); },
  }), /Node\.js 22\.15\.0 or newer/);
});

test('exec failure is reported without a stack trace by the executable entry point', () => {
  const output = spawnSync(process.execPath, [path.join(__dirname, '..', 'bin', 'justmux.cjs'), '--version'], {
    encoding: 'utf8', timeout: 5000,
  });
  assert.equal(output.status, 1);
  assert.match(output.stderr, /^justmux: The native package .* is missing\./);
  assert.doesNotMatch(output.stderr, /\n\s+at /);
});

test('native exec inherits the PID and exit status', (t) => {
  const directory = fs.mkdtempSync(path.join(os.tmpdir(), 'justmux-exec-'));
  t.after(() => fs.rmSync(directory, { recursive: true, force: true }));
  const native = path.join(directory, 'native');
  const script = path.join(directory, 'probe.cjs');
  fs.writeFileSync(native, '#!/bin/sh\nprintf "%s\\n" "$$" "$1"\nexit 23\n', { mode: 0o755 });
  fs.writeFileSync(script, `const {run}=require(${JSON.stringify(path.join(__dirname, '..', 'lib', 'launcher.cjs'))});run(['literal argument'],{resolveExecutable:()=>${JSON.stringify(native)}});`);
  const output = spawnSync(process.execPath, [script], { encoding: 'utf8', timeout: 5000 });
  assert.equal(output.status, 23);
  assert.deepEqual(output.stdout.trim().split('\n'), [String(output.pid), 'literal argument']);
});

test('native signals retain their signal exit semantics', (t) => {
  const directory = fs.mkdtempSync(path.join(os.tmpdir(), 'justmux-signal-'));
  t.after(() => fs.rmSync(directory, { recursive: true, force: true }));
  const script = path.join(directory, 'probe.cjs');
  fs.writeFileSync(script, `const {run}=require(${JSON.stringify(path.join(__dirname, '..', 'lib', 'launcher.cjs'))});run(['-c','kill -TERM $$'],{resolveExecutable:()=>'/bin/sh'});`);
  const output = spawnSync(process.execPath, [script], { encoding: 'utf8', timeout: 5000 });
  assert.equal(output.status, null);
  assert.equal(output.signal, 'SIGTERM');
});
