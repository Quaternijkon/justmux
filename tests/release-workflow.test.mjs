import assert from 'node:assert/strict';
import { createHash } from 'node:crypto';
import { execFileSync } from 'node:child_process';
import fs from 'node:fs/promises';
import os from 'node:os';
import path from 'node:path';
import test from 'node:test';
import { release } from '../scripts/release.mjs';

const VERSION = '0.1.1';
const PLATFORMS = ['darwin-x64', 'darwin-arm64', 'linux-x64', 'linux-arm64'];
const hash = (bytes, algorithm = 'sha256', encoding = 'hex') => (
  createHash(algorithm).update(bytes).digest(encoding)
);

function git(root, ...args) {
  return execFileSync('git', args, {
    cwd: root, encoding: 'utf8', timeout: 15_000,
    stdio: ['ignore', 'pipe', 'pipe'],
  }).trim();
}

function remoteRef(fixture, name) {
  return git(fixture.base, '--git-dir', fixture.origin, 'rev-parse', `${name}^{commit}`);
}

async function fixture(t, { emptyOrigin = false } = {}) {
  const base = await fs.mkdtemp(path.join(os.tmpdir(), 'justmux-release-test-'));
  t.after(() => fs.rm(base, { recursive: true, force: true }));
  const root = path.join(base, 'repo');
  const origin = path.join(base, 'origin.git');
  await fs.mkdir(root);
  git(base, 'init', '--bare', origin);
  git(root, 'init', '-b', 'main');
  git(root, 'config', 'user.name', 'JustMux Release Fixture');
  git(root, 'config', 'user.email', 'release-fixture@example.invalid');
  git(root, 'config', 'commit.gpgsign', 'false');
  git(root, 'config', 'tag.gpgsign', 'false');
  git(root, 'remote', 'add', 'origin', origin);
  await fs.mkdir(path.join(root, 'npm', 'justmux'), { recursive: true });
  await fs.writeFile(path.join(root, '.gitignore'), '/dist/\n/target/\n');
  await fs.writeFile(path.join(root, 'README.md'), 'Release fixture. No real npm or GitHub calls.\n');
  await fs.writeFile(path.join(root, 'Cargo.toml'), '[package]\nname = "justmux"\nversion = "0.1.0"\n');
  await fs.writeFile(path.join(root, 'Cargo.lock'),
    'version = 4\n\n[[package]]\nname = "justmux"\nversion = "0.1.0"\n');
  await fs.writeFile(path.join(root, 'npm', 'justmux', 'package.json'), `${JSON.stringify({
    name: 'justmux', version: '0.1.0', private: true,
    optionalDependencies: Object.fromEntries(PLATFORMS.map((platform) => [`justmux-${platform}`, '0.1.0'])),
  }, null, 2)}\n`);
  git(root, 'add', '.');
  git(root, 'commit', '-m', 'Initial fixture');
  if (!emptyOrigin) git(root, 'push', '-u', 'origin', 'main');
  return { base, root, origin, initial: git(root, 'rev-parse', 'HEAD') };
}

async function writeArtifacts(context) {
  const tarballs = path.join(context.out, 'tarballs');
  await fs.mkdir(tarballs, { recursive: true });
  const packages = [];
  for (const name of [...PLATFORMS.map((platform) => `justmux-${platform}`), 'justmux']) {
    // The core orchestrator checks immutable bytes/checksums; native archive
    // contents are covered by npm-packages and real install tests separately.
    const bytes = Buffer.from(`${name}@${context.version}, source ${context.commit}\n`);
    const filename = `${name}-${context.version}.tgz`;
    await fs.writeFile(path.join(tarballs, filename), bytes);
    packages.push({ name, version: context.version, filename,
      sha256: hash(bytes), integrity: `sha512-${hash(bytes, 'sha512', 'base64')}` });
  }
  await fs.writeFile(path.join(context.out, 'npm-release.json'), `${JSON.stringify({
    version: context.version, main: 'justmux', packages,
  }, null, 2)}\n`);
}

function hooksFor(fixture, overrides = {}) {
  const calls = [];
  const contexts = [];
  const hooks = {};
  for (const name of ['preflight', 'check', 'build', 'install', 'publish']) {
    hooks[name] = async (root, context) => {
      assert.equal(root, fixture.root);
      assert.equal(context.version, VERSION);
      assert.equal(context.branch, 'main');
      assert.equal(context.remote, 'origin');
      assert.equal(context.out, path.join(root, 'dist', 'releases', VERSION, 'npm'));
      calls.push(name);
      contexts.push({ name, ...context });
      if (name === 'build') await writeArtifacts(context);
      if (name === 'publish') {
        // Publication may only begin after BOTH branch and tag are visible.
        assert.equal(remoteRef(fixture, 'refs/heads/main'), context.commit);
        assert.equal(remoteRef(fixture, `refs/tags/v${VERSION}`), context.commit);
        assert.equal(git(root, 'rev-parse', 'HEAD'), context.commit);
      }
      if (overrides[name]) await overrides[name](root, context);
    };
  }
  return { hooks, calls, contexts };
}

async function snapshot(root) {
  const files = {};
  for (const file of ['Cargo.toml', 'Cargo.lock', 'npm/justmux/package.json', 'README.md']) {
    files[file] = await fs.readFile(path.join(root, file), 'utf8');
  }
  return { files, head: git(root, 'rev-parse', 'HEAD'), tags: git(root, 'tag'),
    status: git(root, 'status', '--porcelain') };
}

test('release pushes the branch and version tag atomically before publishing', async (t) => {
  const repo = await fixture(t);
  const { hooks, calls, contexts } = hooksFor(repo);
  await release({ root: repo.root, version: VERSION }, hooks);
  assert.deepEqual(calls.filter((name) => name !== 'preflight'), ['check', 'build', 'install', 'publish']);
  const head = git(repo.root, 'rev-parse', 'HEAD');
  assert.notEqual(head, repo.initial);
  assert.equal(remoteRef(repo, 'refs/heads/main'), head);
  assert.equal(remoteRef(repo, `refs/tags/v${VERSION}`), head);
  assert.equal(git(repo.root, 'status', '--porcelain'), '');
  const manifest = JSON.parse(await fs.readFile(path.join(repo.root, 'npm/justmux/package.json'), 'utf8'));
  assert.equal(manifest.version, VERSION);
  assert.ok(Object.values(manifest.optionalDependencies).every((version) => version === VERSION));
  assert.match(await fs.readFile(path.join(repo.root, 'Cargo.toml'), 'utf8'), /version = "0\.1\.1"/);
  assert.match(await fs.readFile(path.join(repo.root, 'Cargo.lock'), 'utf8'), /version = "0\.1\.1"/);
  assert.equal(contexts.find((context) => context.name === 'publish').commit, head);
});

test('a first release creates main and an annotated tag on an empty origin before publishing', async (t) => {
  const repo = await fixture(t, { emptyOrigin: true });
  assert.equal(git(repo.root, 'ls-remote', 'origin'), '');
  const { hooks, calls } = hooksFor(repo, { publish: async (root, context) => {
    assert.equal(git(repo.base, '--git-dir', repo.origin, 'cat-file', '-t', `refs/tags/v${VERSION}`), 'tag');
    assert.equal(remoteRef(repo, 'refs/heads/main'), context.commit);
    assert.equal(remoteRef(repo, `refs/tags/v${VERSION}`), context.commit);
    assert.equal(git(root, 'rev-parse', `refs/tags/v${VERSION}^{commit}`), context.commit);
  } });
  await release({ root: repo.root, version: VERSION }, hooks);
  assert.deepEqual(calls.filter((name) => name !== 'preflight'), ['check', 'build', 'install', 'publish']);
  assert.notEqual(remoteRef(repo, 'refs/heads/main'), repo.initial);
  assert.equal(git(repo.root, 'status', '--porcelain'), '');
});

for (const version of ['0.1.0', '0.0.9']) {
  test(`version ${version} is refused without changing source or Git refs`, async (t) => {
    const repo = await fixture(t);
    const before = await snapshot(repo.root);
    const { hooks, calls } = hooksFor(repo);
    await assert.rejects(release({ root: repo.root, version }, hooks), /version.*greater/i);
    assert.deepEqual(await snapshot(repo.root), before);
    assert.equal(remoteRef(repo, 'refs/heads/main'), repo.initial);
    assert.equal(git(repo.base, '--git-dir', repo.origin, 'tag'), '');
    await assert.rejects(fs.stat(path.join(repo.root, 'dist')), { code: 'ENOENT' });
    assert.deepEqual(calls, []);
  });
}

test('a checkout behind origin/main fails before source mutation, build or publication', async (t) => {
  const repo = await fixture(t);
  const other = path.join(repo.base, 'other');
  git(repo.base, 'clone', '--branch', 'main', repo.origin, other);
  git(other, 'config', 'user.name', 'Other Release Fixture');
  git(other, 'config', 'user.email', 'other-fixture@example.invalid');
  git(other, 'config', 'commit.gpgsign', 'false');
  await fs.appendFile(path.join(other, 'README.md'), 'Remote source advanced independently.\n');
  git(other, 'commit', '-am', 'Advance origin/main');
  git(other, 'push', 'origin', 'main');
  const remoteHead = git(other, 'rev-parse', 'HEAD');
  const before = await snapshot(repo.root);
  const { hooks, calls } = hooksFor(repo);
  await assert.rejects(release({ root: repo.root, version: VERSION }, hooks), /ancestor|fast.forward|merge/i);
  assert.deepEqual(await snapshot(repo.root), before);
  assert.equal(remoteRef(repo, 'refs/heads/main'), remoteHead);
  assert.equal(git(repo.base, '--git-dir', repo.origin, 'tag'), '');
  assert.ok(!calls.includes('build'));
  assert.ok(!calls.includes('publish'));
});

test('a failed check never builds, pushes or publishes', async (t) => {
  const repo = await fixture(t);
  const { hooks, calls } = hooksFor(repo, { check: async () => { throw new Error('fixture check failed'); } });
  await assert.rejects(release({ root: repo.root, version: VERSION }, hooks), /fixture check failed/);
  assert.equal(remoteRef(repo, 'refs/heads/main'), repo.initial);
  assert.equal(git(repo.base, '--git-dir', repo.origin, 'tag'), '');
  assert.ok(!calls.includes('build'));
  assert.ok(!calls.includes('publish'));
});

test('a rejected release commit resumes the staged source without repeating version preparation', async (t) => {
  const repo = await fixture(t);
  const hook = path.join(repo.root, '.git', 'hooks', 'pre-commit');
  await fs.writeFile(hook, '#!/bin/sh\necho "fixture commit rejected" >&2\nexit 1\n', { mode: 0o755 });
  const { hooks, calls } = hooksFor(repo);
  await assert.rejects(release({ root: repo.root, version: VERSION }, hooks), /commit/i);
  const journal = path.join(repo.root, 'dist', 'releases', VERSION, 'release-state.json');
  const state = JSON.parse(await fs.readFile(journal, 'utf8'));
  assert.equal(state.phase, 'preparing');
  assert.equal(state.baseCommit, repo.initial);
  assert.equal(state.expectedTree, git(repo.root, 'write-tree'));
  assert.equal(git(repo.root, 'rev-parse', 'HEAD'), repo.initial);
  assert.equal(remoteRef(repo, 'refs/heads/main'), repo.initial);
  assert.ok(!calls.includes('check'));
  assert.ok(!calls.includes('build'));
  assert.ok(!calls.includes('publish'));

  await fs.rm(hook);
  await release({ root: repo.root, version: VERSION, resume: true }, hooks);
  const head = git(repo.root, 'rev-parse', 'HEAD');
  assert.equal(git(repo.root, 'rev-parse', `${head}^`), repo.initial);
  assert.equal(git(repo.root, 'rev-parse', `${head}^{tree}`), state.expectedTree);
  assert.equal(remoteRef(repo, 'refs/heads/main'), head);
  assert.equal(remoteRef(repo, `refs/tags/v${VERSION}`), head);
  assert.equal(git(repo.root, 'rev-list', '--count', 'HEAD'), '2');
  assert.equal(git(repo.root, 'status', '--porcelain'), '');
  assert.equal(calls.filter((name) => name === 'publish').length, 1);
});

test('resume adopts a completed commit when its preparing journal was not advanced', async (t) => {
  const repo = await fixture(t);
  const hook = path.join(repo.root, '.git', 'hooks', 'pre-commit');
  await fs.writeFile(hook, '#!/bin/sh\nexit 1\n', { mode: 0o755 });
  const { hooks, calls } = hooksFor(repo);
  await assert.rejects(release({ root: repo.root, version: VERSION }, hooks), /commit/i);
  const journal = path.join(repo.root, 'dist', 'releases', VERSION, 'release-state.json');
  const state = JSON.parse(await fs.readFile(journal, 'utf8'));
  assert.equal(state.phase, 'preparing');
  await fs.rm(hook);
  // Reproduce process termination after git commit succeeds, before the
  // orchestrator can save phase prepared. Its durable journal stays untouched.
  git(repo.root, 'commit', '-m', `Release v${VERSION}`);
  const completedCommit = git(repo.root, 'rev-parse', 'HEAD');
  assert.equal(git(repo.root, 'rev-parse', `${completedCommit}^`), state.baseCommit);
  assert.equal(git(repo.root, 'rev-parse', `${completedCommit}^{tree}`), state.expectedTree);
  assert.equal(JSON.parse(await fs.readFile(journal, 'utf8')).phase, 'preparing');

  await release({ root: repo.root, version: VERSION, resume: true }, hooks);
  assert.equal(git(repo.root, 'rev-parse', 'HEAD'), completedCommit);
  assert.equal(git(repo.root, 'rev-list', '--count', 'HEAD'), '2');
  assert.equal(remoteRef(repo, 'refs/heads/main'), completedCommit);
  assert.equal(remoteRef(repo, `refs/tags/v${VERSION}`), completedCommit);
  assert.equal(calls.filter((name) => name === 'build').length, 1);
  assert.equal(calls.filter((name) => name === 'publish').length, 1);
});

for (const change of ['staged', 'unstaged', 'untracked']) {
  test(`preparing resume refuses ${change} source changes after a rejected commit`, async (t) => {
    const repo = await fixture(t);
    const hook = path.join(repo.root, '.git', 'hooks', 'pre-commit');
    await fs.writeFile(hook, '#!/bin/sh\nexit 1\n', { mode: 0o755 });
    const { hooks, calls } = hooksFor(repo);
    await assert.rejects(release({ root: repo.root, version: VERSION }, hooks), /commit/i);
    await fs.rm(hook);
    if (change === 'untracked') {
      await fs.writeFile(path.join(repo.root, 'unexpected-source.txt'), 'Unexpected new source.\n');
    } else {
      await fs.appendFile(path.join(repo.root, 'README.md'), 'Unexpected source edit.\n');
      if (change === 'staged') git(repo.root, 'add', 'README.md');
    }
    const before = await snapshot(repo.root);
    await assert.rejects(release({ root: repo.root, version: VERSION, resume: true }, hooks), /changed|source|tree|index|untracked/i);
    assert.deepEqual(await snapshot(repo.root), before);
    assert.equal(git(repo.root, 'rev-parse', 'HEAD'), repo.initial);
    assert.equal(remoteRef(repo, 'refs/heads/main'), repo.initial);
    assert.ok(!calls.includes('check'));
    assert.ok(!calls.includes('build'));
    assert.ok(!calls.includes('publish'));
  });
}

test('an atomic push rejection leaves both remote refs untouched and never publishes', async (t) => {
  const repo = await fixture(t);
  const rejectTag = path.join(repo.origin, 'hooks', 'update');
  // A non-atomic push could update main even when this tag is rejected.
  await fs.writeFile(rejectTag, '#!/bin/sh\ncase "$1" in refs/tags/*) exit 1 ;; esac\nexit 0\n', { mode: 0o755 });
  const { hooks, calls } = hooksFor(repo);
  await assert.rejects(release({ root: repo.root, version: VERSION }, hooks), /push|reject|atomic/i);
  assert.equal(remoteRef(repo, 'refs/heads/main'), repo.initial);
  assert.equal(git(repo.base, '--git-dir', repo.origin, 'tag'), '');
  assert.ok(calls.includes('install'));
  assert.ok(!calls.includes('publish'));
});

test('dirty source is refused by default without altering version files or remote refs', async (t) => {
  const repo = await fixture(t);
  await fs.appendFile(path.join(repo.root, 'README.md'), 'Uncommitted edit.\n');
  const before = await snapshot(repo.root);
  const { hooks, calls } = hooksFor(repo);
  await assert.rejects(release({ root: repo.root, version: VERSION }, hooks), /dirty|clean|uncommitted|commit/i);
  assert.deepEqual(await snapshot(repo.root), before);
  assert.equal(remoteRef(repo, 'refs/heads/main'), repo.initial);
  assert.ok(!calls.includes('build'));
  assert.ok(!calls.includes('publish'));
});

test('explicit commitChanges includes source edits in the published source commit', async (t) => {
  const repo = await fixture(t);
  await fs.appendFile(path.join(repo.root, 'README.md'), 'Intended release edit.\n');
  await fs.writeFile(path.join(repo.root, 'new-source.txt'), 'New file included by explicit opt-in.\n');
  const { hooks } = hooksFor(repo);
  await release({ root: repo.root, version: VERSION, commitChanges: true }, hooks);
  const head = git(repo.root, 'rev-parse', 'HEAD');
  assert.match(git(repo.root, 'show', `${head}:README.md`), /Intended release edit/);
  assert.match(git(repo.root, 'show', `${head}:new-source.txt`), /explicit opt-in/);
  assert.equal(remoteRef(repo, `refs/tags/v${VERSION}`), head);
  assert.equal(git(repo.root, 'status', '--porcelain'), '');
});

test('an existing tag for the requested version is refused', async (t) => {
  const repo = await fixture(t);
  git(repo.root, 'tag', `v${VERSION}`, repo.initial);
  const before = await snapshot(repo.root);
  const { hooks, calls } = hooksFor(repo);
  await assert.rejects(release({ root: repo.root, version: VERSION }, hooks), /tag.*exist|exist.*tag|tag.*conflict/i);
  assert.deepEqual(await snapshot(repo.root), before);
  assert.equal(remoteRef(repo, 'refs/heads/main'), repo.initial);
  assert.ok(!calls.includes('publish'));
});

test('failed npm publication resumes the same commit and exact tarballs without rebuilding', async (t) => {
  const repo = await fixture(t);
  let publishAttempts = 0;
  const { hooks, calls } = hooksFor(repo, { publish: async () => {
    publishAttempts++;
    if (publishAttempts === 1) throw new Error('fixture npm unavailable');
  } });
  await assert.rejects(release({ root: repo.root, version: VERSION }, hooks), /fixture npm unavailable/);
  const head = git(repo.root, 'rev-parse', 'HEAD');
  const out = path.join(repo.root, 'dist', 'releases', VERSION, 'npm');
  const index = await fs.readFile(path.join(out, 'npm-release.json'), 'utf8');
  const records = JSON.parse(index).packages;
  const before = await Promise.all(records.map(async (record) => (
    hash(await fs.readFile(path.join(out, 'tarballs', record.filename)))
  )));
  await release({ root: repo.root, version: VERSION, resume: true }, hooks);
  assert.equal(git(repo.root, 'rev-parse', 'HEAD'), head);
  assert.equal(remoteRef(repo, 'refs/heads/main'), head);
  assert.equal(remoteRef(repo, `refs/tags/v${VERSION}`), head);
  assert.equal(await fs.readFile(path.join(out, 'npm-release.json'), 'utf8'), index);
  assert.deepEqual(await Promise.all(records.map(async (record) => (
    hash(await fs.readFile(path.join(out, 'tarballs', record.filename)))
  ))), before);
  assert.equal(calls.filter((name) => name === 'build').length, 1);
  assert.equal(calls.filter((name) => name === 'install').length, 1);
  assert.equal(publishAttempts, 2);
});

test('resume refuses source changes after the release commit', async (t) => {
  const repo = await fixture(t);
  const { hooks, calls } = hooksFor(repo, { publish: async () => { throw new Error('fixture npm unavailable'); } });
  await assert.rejects(release({ root: repo.root, version: VERSION }, hooks), /fixture npm unavailable/);
  await fs.appendFile(path.join(repo.root, 'README.md'), 'Changed after release build.\n');
  git(repo.root, 'add', 'README.md');
  git(repo.root, 'commit', '-m', 'Change source after failed publication');
  const publishCount = calls.filter((name) => name === 'publish').length;
  await assert.rejects(release({ root: repo.root, version: VERSION, resume: true }, hooks), /commit|source|HEAD|changed/i);
  assert.equal(calls.filter((name) => name === 'publish').length, publishCount);
  assert.equal(calls.filter((name) => name === 'build').length, 1);
});

test('resume refuses a changed tarball instead of publishing different bytes', async (t) => {
  const repo = await fixture(t);
  const { hooks, calls } = hooksFor(repo, { publish: async () => { throw new Error('fixture npm unavailable'); } });
  await assert.rejects(release({ root: repo.root, version: VERSION }, hooks), /fixture npm unavailable/);
  const out = path.join(repo.root, 'dist', 'releases', VERSION, 'npm');
  const index = JSON.parse(await fs.readFile(path.join(out, 'npm-release.json'), 'utf8'));
  await fs.appendFile(path.join(out, 'tarballs', index.packages[0].filename), 'tampered bytes');
  const publishCount = calls.filter((name) => name === 'publish').length;
  await assert.rejects(release({ root: repo.root, version: VERSION, resume: true }, hooks), /checksum|integrity|artifact|changed|hash/i);
  assert.equal(calls.filter((name) => name === 'publish').length, publishCount);
  assert.equal(calls.filter((name) => name === 'build').length, 1);
});

test('dry-run changes neither source, local Git refs, remote Git refs nor output files', async (t) => {
  const repo = await fixture(t);
  const before = await snapshot(repo.root);
  const { hooks, calls } = hooksFor(repo);
  await release({ root: repo.root, version: VERSION, dryRun: true }, hooks);
  assert.deepEqual(await snapshot(repo.root), before);
  assert.equal(remoteRef(repo, 'refs/heads/main'), repo.initial);
  assert.equal(git(repo.base, '--git-dir', repo.origin, 'tag'), '');
  await assert.rejects(fs.stat(path.join(repo.root, 'dist')), { code: 'ENOENT' });
  assert.deepEqual(calls, [], 'dry-run must not run checks, build, installation or publication hooks');
});
