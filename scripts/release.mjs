#!/usr/bin/env node
/** Checked, resumable local releases. Running this CLI intentionally pushes and publishes. */
import { spawnSync } from 'node:child_process';
import { createHash, randomUUID } from 'node:crypto';
import { closeSync, existsSync, lstatSync, mkdirSync, openSync, readFileSync, readdirSync, renameSync, unlinkSync, writeFileSync } from 'node:fs';
import path from 'node:path';
import { fileURLToPath, pathToFileURL } from 'node:url';
import { compareVersions, SUPPORTED_PLATFORMS } from './npm-packages.mjs';

const ROOT = path.resolve(path.dirname(fileURLToPath(import.meta.url)), '..');
const PHASES = ['preparing', 'prepared', 'checked', 'built', 'tested', 'tagged', 'pushed', 'published'];
const VERSION = /^(0|[1-9]\d*)\.(0|[1-9]\d*)\.(0|[1-9]\d*)(?:-[0-9A-Za-z-]+(?:\.[0-9A-Za-z-]+)*)?$/;

function command(root, executable, args, { capture = false, timeout = 30 * 60_000 } = {}) {
  const result = spawnSync(executable, args, {
    cwd: root, encoding: 'utf8', shell: false,
    stdio: capture ? ['ignore', 'pipe', 'pipe'] : 'inherit', timeout,
  });
  if (result.error || result.status !== 0) {
    throw new Error(`${executable} ${args.join(' ')} failed: ${result.error?.message ?? result.stderr?.trim() ?? result.signal ?? result.status}`);
  }
  return result.stdout?.trim() ?? '';
}

function git(root, args) { return command(root, 'git', args, { capture: true, timeout: 120_000 }); }
function sourceVersion(root) {
  return readFileSync(path.join(root, 'Cargo.toml'), 'utf8')
    .match(/^\[package\][\s\S]*?^version\s*=\s*"([^"]+)"/m)?.[1];
}
function clean(root) { return git(root, ['status', '--porcelain', '--untracked-files=all']) === ''; }
function head(root) { return git(root, ['rev-parse', 'HEAD']); }
function digest(file) { return createHash('sha256').update(readFileSync(file)).digest('hex'); }

function syncVersion(root, version) {
  const tomlPath = path.join(root, 'Cargo.toml');
  const lockPath = path.join(root, 'Cargo.lock');
  const manifestPath = path.join(root, 'npm/justmux/package.json');
  const current = sourceVersion(root);
  const manifest = JSON.parse(readFileSync(manifestPath, 'utf8'));
  if (!current || manifest.version !== current) throw new Error('Cargo and npm source versions disagree');
  const nextToml = readFileSync(tomlPath, 'utf8').replace(
    /(^\[package\][\s\S]*?^version\s*=\s*")[^"]+("[ \t]*$)/m,
    (_match, before, after) => `${before}${version}${after}`,
  );
  let lockMatches = 0;
  const nextLock = readFileSync(lockPath, 'utf8').replace(
    /(\[\[package\]\]\r?\nname = "justmux"\r?\nversion = ")[^"]+("\r?\n)/g,
    (_match, before, after) => { lockMatches += 1; return `${before}${version}${after}`; },
  );
  if (lockMatches !== 1) throw new Error('Cargo.lock must have exactly one local justmux package');
  manifest.version = version;
  for (const name of Object.keys(manifest.optionalDependencies ?? {})) manifest.optionalDependencies[name] = version;
  writeFileSync(tomlPath, nextToml);
  writeFileSync(lockPath, nextLock);
  writeFileSync(manifestPath, `${JSON.stringify(manifest, null, 2)}\n`);
}

function acquireLock(root) {
  const directory = path.resolve(root, git(root, ['rev-parse', '--git-common-dir']));
  const filename = path.join(directory, 'justmux-release.lock');
  const token = randomUUID();
  for (let attempt = 0; attempt < 2; attempt += 1) {
    try {
      const descriptor = openSync(filename, 'wx', 0o600);
      try { writeFileSync(descriptor, JSON.stringify({ pid: process.pid, token })); }
      finally { closeSync(descriptor); }
      return () => {
        if (JSON.parse(readFileSync(filename, 'utf8')).token === token) unlinkSync(filename);
      };
    } catch (error) {
      if (error.code !== 'EEXIST') throw error;
      let owner;
      try { owner = JSON.parse(readFileSync(filename, 'utf8')); }
      catch { throw new Error(`Unreadable release lock: ${filename}. Inspect it before removing it.`); }
      if (!Number.isInteger(owner.pid) || owner.pid <= 0) throw new Error(`Invalid release lock: ${filename}`);
      try { process.kill(owner.pid, 0); }
      catch (probe) {
        if (probe.code === 'ESRCH') { unlinkSync(filename); continue; }
        throw new Error(`Cannot inspect release lock owned by PID ${owner.pid}`);
      }
      throw new Error(`Another release is running (PID ${owner.pid})`);
    }
  }
  throw new Error('Could not acquire release lock; retry after inspecting concurrent releases');
}

function artifactSnapshot(out, version) {
  if (existsSync(path.join(out, '.build-incomplete'))) throw new Error('Release build is incomplete');
  const indexPath = path.join(out, 'npm-release.json');
  const index = JSON.parse(readFileSync(indexPath, 'utf8'));
  if (index.version !== version || !Array.isArray(index.packages) || index.packages.length === 0) {
    throw new Error('Invalid artifact manifest version or package list');
  }
  const files = { 'npm-release.json': digest(indexPath) };
  const names = new Set();
  for (const item of index.packages) {
    if (!item.filename || path.basename(item.filename) !== item.filename || names.has(item.filename)
      || item.version !== version) throw new Error('Invalid or duplicate package artifact');
    names.add(item.filename);
    const filename = path.join(out, 'tarballs', item.filename);
    if (!lstatSync(filename).isFile() || digest(filename) !== item.sha256) throw new Error(`Artifact hash mismatch: ${item.filename}`);
  }
  // Include checksums and any additional tarball files, so additions and edits
  // cannot silently change what a resumed publisher reads.
  for (const filename of readdirSync(path.join(out, 'tarballs')).sort()) {
    const fullPath = path.join(out, 'tarballs', filename);
    if (!lstatSync(fullPath).isFile()) throw new Error(`Unexpected artifact entry: ${filename}`);
    files[`tarballs/${filename}`] = digest(fullPath);
  }
  return files;
}

function verifySource(root, context) {
  if (head(root) !== context.commit || !clean(root) || sourceVersion(root) !== context.version) {
    throw new Error('Release source changed: expected the recorded clean commit and version');
  }
}

function verifyPreparing(root, context, state) {
  if (!state.baseCommit || !state.expectedTree || sourceVersion(root) !== context.version) {
    throw new Error('Preparing release journal is inconsistent');
  }
  if (git(root, ['tag', '--list', `v${context.version}`])) throw new Error('Preparing release already has a version tag');
  const current = head(root);
  if (current === state.baseCommit) {
    // Commit hooks and signing may fail. Retry only the exact index already
    // approved for this release, with no unstaged changes or new source files.
    if (git(root, ['write-tree']) !== state.expectedTree
      || git(root, ['diff', '--name-only', '--ignore-submodules=none']) !== ''
      || git(root, ['ls-files', '--others', '--exclude-standard']) !== '') {
      throw new Error('Preparing release source or index changed; expected the recorded staged tree');
    }
    return 'commit';
  }
  const history = git(root, ['rev-list', '--parents', '-n', '1', 'HEAD']).split(/\s+/);
  if (history.length !== 2 || history[1] !== state.baseCommit
    || git(root, ['rev-parse', 'HEAD^{tree}']) !== state.expectedTree || !clean(root)) {
    throw new Error('Preparing release cannot adopt an unexpected commit or changed source');
  }
  // The commit can succeed immediately before an interruption prevents the
  // next journal write. Adopt that one matching child instead of duplicating it.
  return 'adopt';
}

function tagCommit(root, tag) {
  const refs = git(root, ['tag', '--list', tag]);
  if (!refs) return undefined;
  if (git(root, ['cat-file', '-t', `refs/tags/${tag}`]) !== 'tag') throw new Error(`Release tag ${tag} must be annotated`);
  return git(root, ['rev-parse', `refs/tags/${tag}^{commit}`]);
}

function verifyTag(root, context, required = false) {
  const existing = tagCommit(root, `v${context.version}`);
  if (existing && existing !== context.commit) throw new Error('Release tag points to a different commit');
  if (required && !existing) throw new Error('Recorded release tag is missing');
  return existing;
}

function verifyArtifacts(context, state) {
  const actual = artifactSnapshot(context.out, context.version);
  if (JSON.stringify(actual) !== JSON.stringify(state.artifacts)) throw new Error('Release artifacts changed since they were built');
}

function remotePreflight(root, context, state) {
  git(root, ['var', 'GIT_AUTHOR_IDENT']);
  git(root, ['var', 'GIT_COMMITTER_IDENT']);
  const remoteUrl = git(root, ['remote', 'get-url', context.remote]);
  const remoteRefs = git(root, ['ls-remote', context.remote, 'refs/heads/main', `refs/tags/v${context.version}`, `refs/tags/v${context.version}^{}`]);
  const remoteTag = remoteRefs.split('\n').find((line) => line.endsWith(`\trefs/tags/v${context.version}`))?.split('\t')[0];
  if (!state && remoteTag) throw new Error(`Release tag v${context.version} already exists on origin`);
  if (remoteTag && state?.tagObject && remoteTag !== state.tagObject) throw new Error('Remote release tag conflicts with the recorded tag');
  if (state && PHASES.indexOf(state.phase) >= PHASES.indexOf('pushed') && !remoteTag) throw new Error('Previously pushed release tag is missing on origin');
  const main = remoteRefs.split('\n').find((line) => line.endsWith('\trefs/heads/main'))?.split('\t')[0];
  if (main) {
    git(root, ['fetch', '--no-tags', context.remote, 'refs/heads/main']);
    try { git(root, ['merge-base', '--is-ancestor', main, 'HEAD']); }
    catch { throw new Error('origin/main is not an ancestor of this checkout; fast-forward or merge it before releasing'); }
  }
  return remoteUrl;
}

const production = {
  async preflight(root) {
    if (process.platform !== 'darwin' || process.arch !== 'arm64') {
      throw new Error('The complete local release requires macOS ARM64 and the Apple SDK');
    }
    if (compareVersions(process.versions.node, '22.15.0') < 0 || typeof process.execve !== 'function') {
      throw new Error('Node.js 22.15 or newer with process.execve is required; Node.js 24 LTS is recommended');
    }
    for (const [program, args] of [
      ['cargo', ['--version']], ['python3', ['--version']], ['tmux', ['-V']],
      ['zig', ['version']], ['cargo', ['zigbuild', '--help']], ['xcrun', ['--show-sdk-path']],
      ['npm', ['whoami', '--registry', 'https://registry.npmjs.org']],
    ]) command(root, program, args, { capture: true, timeout: 30_000 });
    const targets = command(root, 'rustup', ['target', 'list', '--installed'], { capture: true }).split(/\s+/);
    for (const { target } of SUPPORTED_PLATFORMS) {
      if (!targets.includes(target)) throw new Error(`Missing Rust target: rustup target add ${target}`);
    }
  },
  async check(root, context) {
    for (const args of [
      ['fmt', '--all', '--', '--check'], ['clippy', '--all-targets', '--all-features', '--', '-D', 'warnings'],
      ['test', '--all'], ['build', '--release'],
    ]) command(root, 'cargo', args);
    command(root, 'npm', ['--prefix', 'npm/justmux', 'test']);
    const tests = readdirSync(path.join(root, 'tests')).filter((file) => file.endsWith('.test.mjs')).sort();
    command(root, process.execPath, ['--test', ...tests.map((file) => path.join(root, 'tests', file))]);
    command(root, 'python3', ['scripts/pty_smoke.py', '--cast', path.join(root, 'dist/releases', context.version, 'demo.cast')]);
  },
  async build(root, context) {
    command(root, process.execPath, ['scripts/npm-release.mjs', 'build', '--all', '--out', context.out]);
  },
  async install(root, context) {
    const binary = path.join(context.out, `justmux-${process.platform}-${process.arch}`, 'bin/jm');
    command(root, process.execPath, ['scripts/test-npm-install.mjs', '--binary', binary, '--pty']);
    command(root, process.execPath, ['scripts/npm-packages.mjs', 'publish', '--dir', context.out, '--dry-run']);
  },
  async publish(root, context) {
    command(root, process.execPath, ['scripts/npm-packages.mjs', 'publish', '--dir', context.out]);
    command(root, process.execPath, ['scripts/npm-packages.mjs', 'verify-registry', '--dir', context.out]);
  },
};

/** Hooks replace tool-heavy stages for isolated tests; Git always runs for real. */
export async function release(options, hooks = {}) {
  const root = path.resolve(options.root ?? ROOT);
  const version = options.version;
  if (!VERSION.test(version ?? '') || (version.split('-').slice(1).join('-').split('.').some((part) => /^0\d+$/.test(part)))) {
    throw new Error('Provide a release version such as 0.1.1 or 0.2.0-beta.1');
  }
  const branch = git(root, ['symbolic-ref', '--short', 'HEAD']);
  if (branch !== 'main') throw new Error('Releases must run on branch main');
  const context = { version, out: path.join(root, 'dist/releases', version, 'npm'), commit: head(root), branch, remote: 'origin' };
  const statePath = path.join(root, 'dist/releases', version, 'release-state.json');
  const current = sourceVersion(root);
  if (!options.resume && compareVersions(version, current) <= 0) throw new Error(`Release version must be greater than ${current}`);
  if (!options.resume && !options.commitChanges && !clean(root)) throw new Error('Working tree is dirty; review changes and explicitly use --commit to include all nonignored files');
  if (!options.resume && existsSync(statePath)) throw new Error('Release state already exists; inspect it and use --resume');
  if (options.dryRun) {
    console.log(`Dry run: ${options.resume ? 'resume' : 'prepare'} JustMux ${version} on main; check, build macOS ARM64 and Linux x64, test installation, annotate v${version}, atomically push origin main + tag, then publish and verify npm.`);
    console.log(`Artifacts: ${context.out}\nNo files, Git refs, npm packages, or remote state were changed.`);
    return { ...context, dryRun: true };
  }
  const unlock = acquireLock(root);
  try {
    let state;
    const save = (phase) => {
      state.phase = phase;
      const temporary = `${statePath}.${process.pid}.tmp`;
      writeFileSync(temporary, `${JSON.stringify(state, null, 2)}\n`);
      renameSync(temporary, statePath);
    };
    if (options.resume) {
      state = JSON.parse(readFileSync(statePath, 'utf8'));
      if (state.version !== version || state.branch !== branch || state.remote !== context.remote || !PHASES.includes(state.phase)) {
        throw new Error('Release journal is inconsistent');
      }
      context.commit = state.commit;
      if (state.phase === 'preparing') verifyPreparing(root, context, state);
      else {
        verifySource(root, context);
        verifyTag(root, context, PHASES.indexOf(state.phase) >= PHASES.indexOf('tagged'));
        if (state.tagObject && git(root, ['rev-parse', `refs/tags/v${version}`]) !== state.tagObject) throw new Error('Release tag object changed since preparation');
        if (PHASES.indexOf(state.phase) >= PHASES.indexOf('built')) verifyArtifacts(context, state);
      }
    } else if (git(root, ['tag', '--list', `v${version}`])) throw new Error(`Release tag v${version} already exists`);

    const remoteUrl = remotePreflight(root, context, state);
    if (state?.remoteUrl && remoteUrl !== state.remoteUrl) throw new Error('Release remote URL changed since preparation');
    await (hooks.preflight ?? production.preflight)(root, context);
    if (!state) {
      if (head(root) !== context.commit || (!options.commitChanges && !clean(root))) {
        throw new Error('Release source changed during preflight; review the checkout and retry');
      }
      syncVersion(root, version);
      git(root, options.commitChanges ? ['add', '--all'] : ['add', '--', 'Cargo.toml', 'Cargo.lock', 'npm/justmux/package.json']);
      mkdirSync(path.dirname(statePath), { recursive: true });
      state = {
        version, commit: context.commit, baseCommit: context.commit,
        expectedTree: git(root, ['write-tree']), branch, remote: context.remote, remoteUrl,
      };
      save('preparing');
    }
    if (state.phase === 'preparing') {
      if (verifyPreparing(root, context, state) === 'commit') git(root, ['commit', '-m', `Release v${version}`]);
      // A successful hook must not substitute different staged contents or add
      // another parent. Apply the same adoption checks before accepting HEAD.
      if (verifyPreparing(root, context, state) !== 'adopt') throw new Error('Release commit did not advance HEAD');
      context.commit = head(root);
      state.commit = context.commit;
      verifySource(root, context);
      save('prepared');
    } else {
      // Recheck immediately after preflight, before any checks or builders can
      // consume source modified while authentication/tool probes were running.
      verifySource(root, context);
      verifyTag(root, context, PHASES.indexOf(state.phase) >= PHASES.indexOf('tagged'));
      if (state.tagObject && git(root, ['rev-parse', `refs/tags/v${version}`]) !== state.tagObject) throw new Error('Release tag object changed during preflight');
      if (PHASES.indexOf(state.phase) >= PHASES.indexOf('built')) verifyArtifacts(context, state);
    }
    const before = (phase) => PHASES.indexOf(state.phase) < PHASES.indexOf(phase);
    if (before('checked')) {
      await (hooks.check ?? production.check)(root, context);
      verifySource(root, context);
      save('checked');
    }
    if (before('built')) {
      await (hooks.build ?? production.build)(root, context);
      verifySource(root, context);
      state.artifacts = artifactSnapshot(context.out, version);
      save('built');
    }
    if (before('tested')) {
      await (hooks.install ?? production.install)(root, context);
      verifySource(root, context);
      verifyArtifacts(context, state);
      save('tested');
    }
    verifySource(root, context);
    verifyArtifacts(context, state);
    if (before('tagged')) {
      if (!verifyTag(root, context)) git(root, ['tag', '-a', `v${version}`, '-m', `JustMux ${version}`, context.commit]);
      state.tagObject = git(root, ['rev-parse', `refs/tags/v${version}`]);
      save('tagged');
    }
    verifyTag(root, context, true);
    if (before('pushed')) {
      // Both refs advance together or neither does. No force, ref deletion, or
      // rollback: another writer's newer remote branch must make this fail.
      git(root, ['push', '--atomic', context.remote, 'HEAD:refs/heads/main', `refs/tags/v${version}`]);
      save('pushed');
    }
    if (before('published')) {
      verifySource(root, context);
      verifyArtifacts(context, state);
      await (hooks.publish ?? production.publish)(root, context);
      verifySource(root, context);
      verifyArtifacts(context, state);
      save('published');
    }
    console.log(`JustMux ${version} release ${state.phase}: ${context.commit}`);
    return { ...context, phase: state.phase };
  } finally { unlock(); }
}

function argumentsFrom(argv) {
  const options = {};
  for (const argument of argv) {
    if (argument === '--resume') options.resume = true;
    else if (argument === '--dry-run') options.dryRun = true;
    else if (argument === '--commit') options.commitChanges = true;
    else if (argument === '--help' || argument === '-h') options.help = true;
    else if (!argument.startsWith('-') && !options.version) options.version = argument;
    else throw new Error(`Unknown argument: ${argument}`);
  }
  return options;
}

if (process.argv[1] && import.meta.url === pathToFileURL(path.resolve(process.argv[1])).href) {
  try {
    const options = argumentsFrom(process.argv.slice(2));
    if (options.help || !options.version) console.log('Usage: node scripts/release.mjs VERSION [--commit] [--resume] [--dry-run]\nWithout --dry-run this command creates a release commit/tag, pushes main and tag, and publishes to npm.');
    else await release(options);
  } catch (error) { console.error(`release: ${error.message}`); process.exitCode = 1; }
}
