'use strict';

const assert = require('node:assert/strict');
const { spawnSync } = require('node:child_process');
const fs = require('node:fs');
const os = require('node:os');
const path = require('node:path');
const test = require('node:test');
const {
  ensureGlobalLinks,
  userPathDirs,
  classifySlot,
  defaultAllowedDirs,
  resolveCommandInPath,
} = require('../lib/postinstall.cjs');

const MANIFEST = {
  name: 'justmux',
  version: '0.4.1',
  bin: {
    jm: 'bin/justmux.cjs',
    justmux: 'bin/justmux.cjs',
  },
};

function setupInstall(t) {
  const root = fs.realpathSync(fs.mkdtempSync(path.join(os.tmpdir(), 'justmux-postinstall-')));
  t.after(() => fs.rmSync(root, { recursive: true, force: true }));

  const prefix = path.join(root, 'npm-prefix');
  const packageRoot = path.join(prefix, 'lib', 'node_modules', 'justmux');
  const prefixBin = path.join(prefix, 'bin');
  const home = path.join(root, 'home');
  const localBin = path.join(home, '.local', 'bin');
  const launcherPath = path.join(packageRoot, 'bin', 'justmux.cjs');

  fs.mkdirSync(path.dirname(launcherPath), { recursive: true });
  fs.mkdirSync(prefixBin, { recursive: true });
  fs.mkdirSync(localBin, { recursive: true });
  fs.writeFileSync(path.join(packageRoot, 'package.json'), JSON.stringify(MANIFEST));
  fs.writeFileSync(
    launcherPath,
    '#!/usr/bin/env node\nconst path = require("node:path");\nconsole.log(path.basename(process.argv[1]), ...process.argv.slice(2));\n',
    { mode: 0o755 },
  );

  return { root, prefix, packageRoot, prefixBin, home, localBin, launcherPath };
}

function linkPrefixBins(fixture) {
  const target = path.relative(fixture.prefixBin, fixture.launcherPath);
  for (const cmd of Object.keys(MANIFEST.bin)) {
    fs.symlinkSync(target, path.join(fixture.prefixBin, cmd));
  }
}

test('already on PATH leaves allowlisted user bin directories untouched', (t) => {
  const fixture = setupInstall(t);
  linkPrefixBins(fixture);

  const result = ensureGlobalLinks({
    packageRoot: fixture.packageRoot,
    runtime: {
      platform: 'linux',
      env: {
        HOME: fixture.home,
        npm_config_global: 'true',
        PATH: `${fixture.prefixBin}${path.delimiter}${fixture.localBin}`,
      },
    },
    allowedDirs: [fixture.localBin],
  });

  assert.equal(result.action, 'already-on-path');
  assert.deepEqual(result.linked, []);
  assert.equal(fs.existsSync(path.join(fixture.localBin, 'jm')), false);
  assert.equal(fs.existsSync(path.join(fixture.localBin, 'justmux')), false);
});

test('missing prefix bin from PATH links jm and justmux into writable allowlisted PATH directory', (t) => {
  const fixture = setupInstall(t);
  linkPrefixBins(fixture);

  const result = ensureGlobalLinks({
    packageRoot: fixture.packageRoot,
    runtime: {
      platform: 'linux',
      env: {
        HOME: fixture.home,
        npm_config_global: 'true',
        PATH: `${fixture.localBin}${path.delimiter}/usr/bin${path.delimiter}/bin`,
      },
    },
    allowedDirs: [fixture.localBin],
  });

  assert.equal(result.action, 'linked');
  assert.deepEqual(result.linked.map((entry) => entry.cmd), ['jm', 'justmux']);
  for (const cmd of ['jm', 'justmux']) {
    const linkPath = path.join(fixture.localBin, cmd);
    assert.equal(fs.lstatSync(linkPath).isSymbolicLink(), true);
    assert.equal(fs.readlinkSync(linkPath), fixture.launcherPath);
    const out = spawnSync(linkPath, ['--version'], { encoding: 'utf8' });
    assert.equal(out.status, 0);
    assert.equal(out.stdout.trim(), `${cmd} --version`);
  }
});

test('bin-links=false simulation repairs <prefix>/bin without touching $HOME/.local/bin when prefix bin is on PATH', (t) => {
  const fixture = setupInstall(t);
  // Deliberately omit prefixBin symlinks to simulate npm --bin-links=false.
  const result = ensureGlobalLinks({
    packageRoot: fixture.packageRoot,
    runtime: {
      platform: 'darwin',
      env: {
        HOME: fixture.home,
        npm_config_global: 'true',
        PATH: `${fixture.prefixBin}${path.delimiter}${fixture.localBin}`,
      },
    },
    allowedDirs: [fixture.localBin],
  });

  assert.equal(result.action, 'linked');
  assert.deepEqual(result.prefixLinked.map((entry) => entry.cmd), ['jm', 'justmux']);
  assert.deepEqual(result.linked, []);
  assert.equal(fs.existsSync(path.join(fixture.localBin, 'jm')), false);
  assert.equal(fs.existsSync(path.join(fixture.localBin, 'justmux')), false);
  for (const cmd of ['jm', 'justmux']) {
    const out = spawnSync(path.join(fixture.prefixBin, cmd), ['doctor'], { encoding: 'utf8' });
    assert.equal(out.status, 0);
    assert.equal(out.stdout.trim(), `${cmd} doctor`);
  }
});

test('non-global installs are strict no-ops', (t) => {
  const fixture = setupInstall(t);
  const localProjectPkg = path.join(fixture.root, 'project', 'node_modules', 'justmux');
  fs.mkdirSync(path.join(localProjectPkg, 'bin'), { recursive: true });
  fs.writeFileSync(path.join(localProjectPkg, 'package.json'), JSON.stringify(MANIFEST));
  fs.writeFileSync(path.join(localProjectPkg, 'bin', 'justmux.cjs'), '#!/usr/bin/env node\n', { mode: 0o755 });

  for (const [pkgRoot, globalVal] of [
    [localProjectPkg, 'true'],
    [fixture.packageRoot, 'false'],
    [fixture.packageRoot, undefined],
  ]) {
    const result = ensureGlobalLinks({
      packageRoot: pkgRoot,
      runtime: {
        platform: 'linux',
        env: {
          HOME: fixture.home,
          ...(globalVal !== undefined ? { npm_config_global: globalVal } : {}),
          PATH: fixture.localBin,
        },
      },
      allowedDirs: [fixture.localBin],
    });
    assert.equal(result.action, 'skip');
    assert.equal(result.reason, 'not-global');
  }
  assert.equal(fs.existsSync(path.join(fixture.localBin, 'jm')), false);
});

test('JUSTMUX_NO_POSTINSTALL_LINK=1 opts out cleanly', (t) => {
  const fixture = setupInstall(t);
  const result = ensureGlobalLinks({
    packageRoot: fixture.packageRoot,
    runtime: {
      platform: 'linux',
      env: {
        HOME: fixture.home,
        npm_config_global: 'true',
        JUSTMUX_NO_POSTINSTALL_LINK: '1',
        PATH: fixture.localBin,
      },
    },
    allowedDirs: [fixture.localBin],
  });

  assert.deepEqual(result, { action: 'skip', reason: 'opt-out' });
  assert.equal(fs.existsSync(path.join(fixture.prefixBin, 'jm')), false);
  assert.equal(fs.existsSync(path.join(fixture.localBin, 'jm')), false);
});

test('foreign jm binary on PATH or inside allowedDir is never overwritten or shadowed while justmux still links', (t) => {
  const fixture = setupInstall(t);
  linkPrefixBins(fixture);

  // Case 1: foreign `jm` in an earlier PATH directory (not in allowedDirs).
  const customBin = path.join(fixture.root, 'custom-bin');
  fs.mkdirSync(customBin, { recursive: true });
  const foreignInPath = path.join(customBin, 'jm');
  fs.writeFileSync(foreignInPath, '#!/bin/sh\necho foreign-jm\n', { mode: 0o755 });

  const first = ensureGlobalLinks({
    packageRoot: fixture.packageRoot,
    runtime: {
      platform: 'linux',
      env: {
        HOME: fixture.home,
        npm_config_global: 'true',
        PATH: `${customBin}${path.delimiter}${fixture.localBin}`,
      },
    },
    allowedDirs: [fixture.localBin],
  });

  assert.equal(fs.readFileSync(foreignInPath, 'utf8'), '#!/bin/sh\necho foreign-jm\n');
  assert.equal(fs.existsSync(path.join(fixture.localBin, 'jm')), false);
  assert.equal(fs.readlinkSync(path.join(fixture.localBin, 'justmux')), fixture.launcherPath);
  assert.ok(first.skipped.some((entry) => entry.cmd === 'jm' && entry.reason === 'foreign-command'));

  // Case 2: foreign regular file directly inside allowedDir (`localBin/jm`).
  fs.rmSync(path.join(fixture.localBin, 'justmux'), { force: true });
  const foreignInAllowed = path.join(fixture.localBin, 'jm');
  fs.writeFileSync(foreignInAllowed, '#!/bin/sh\necho local-foreign-jm\n', { mode: 0o755 });

  const second = ensureGlobalLinks({
    packageRoot: fixture.packageRoot,
    runtime: {
      platform: 'linux',
      env: {
        HOME: fixture.home,
        npm_config_global: 'true',
        PATH: fixture.localBin,
      },
    },
    allowedDirs: [fixture.localBin],
  });

  assert.equal(fs.lstatSync(foreignInAllowed).isSymbolicLink(), false);
  assert.equal(fs.readFileSync(foreignInAllowed, 'utf8'), '#!/bin/sh\necho local-foreign-jm\n');
  assert.equal(fs.readlinkSync(path.join(fixture.localBin, 'justmux')), fixture.launcherPath);
  assert.ok(second.skipped.some((entry) => entry.cmd === 'jm' && entry.reason === 'foreign-command'));
});

test('stale or broken JustMux symlink is updated in place', (t) => {
  const fixture = setupInstall(t);
  linkPrefixBins(fixture);

  const staleTarget = path.join(fixture.root, 'old-node', 'lib', 'node_modules', 'justmux', 'bin', 'justmux.cjs');
  const jmLink = path.join(fixture.localBin, 'jm');
  const justmuxLink = path.join(fixture.localBin, 'justmux');
  fs.symlinkSync(staleTarget, jmLink);
  fs.symlinkSync(staleTarget, justmuxLink);

  assert.equal(classifySlot(jmLink, { launcherPath: fixture.launcherPath, prefixBin: fixture.prefixBin, cmd: 'jm' }), 'managed');

  const result = ensureGlobalLinks({
    packageRoot: fixture.packageRoot,
    runtime: {
      platform: 'linux',
      env: {
        HOME: fixture.home,
        npm_config_global: 'true',
        PATH: fixture.localBin,
      },
    },
    allowedDirs: [fixture.localBin],
  });

  assert.equal(result.action, 'linked');
  assert.equal(fs.readlinkSync(jmLink), fixture.launcherPath);
  assert.equal(fs.readlinkSync(justmuxLink), fixture.launcherPath);
});

test('userPathDirs ignores npm-injected synthetic bin directories and relative entries', () => {
  const parsed = userPathDirs([
    '',
    'relative/bin',
    '/tmp/pkg/node_modules/.bin',
    '/opt/node/lib/node_modules/npm/node_modules/@npmcli/run-script/lib/node-gyp-bin',
    '/usr/local/bin/',
    '/home/user/.local/bin',
    '/usr/local/bin',
  ].join(path.delimiter));

  assert.deepEqual(parsed, ['/usr/local/bin', '/home/user/.local/bin']);
  assert.deepEqual(defaultAllowedDirs({ HOME: '/home/tester' }), [
    '/usr/local/bin',
    '/opt/homebrew/bin',
    '/home/tester/.local/bin',
    '/home/tester/bin',
  ]);
  assert.equal(resolveCommandInPath('definitely-nonexistent-justmux-cmd', parsed), null);
});

test('unwritable allowed directory fails gracefully without throwing', (t) => {
  const fixture = setupInstall(t);
  linkPrefixBins(fixture);

  const wrappedFs = {
    ...fs,
    accessSync(target, mode) {
      if (target === fixture.localBin && mode === (fs.constants.W_OK | fs.constants.X_OK)) {
        const err = new Error('permission denied');
        err.code = 'EACCES';
        throw err;
      }
      return fs.accessSync(target, mode);
    },
  };

  const result = ensureGlobalLinks({
    packageRoot: fixture.packageRoot,
    fs: wrappedFs,
    runtime: {
      platform: 'linux',
      env: {
        HOME: fixture.home,
        npm_config_global: 'true',
        PATH: fixture.localBin,
      },
    },
    allowedDirs: [fixture.localBin],
  });

  assert.equal(result.action, 'already-on-path');
  assert.deepEqual(result.linked, []);
  assert.deepEqual(result.skipped, [
    { cmd: 'jm', reason: 'no-writable-path-dir' },
    { cmd: 'justmux', reason: 'no-writable-path-dir' },
  ]);
});
