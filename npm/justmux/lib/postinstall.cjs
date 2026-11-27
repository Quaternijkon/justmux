'use strict';

const nativeFs = require('node:fs');
const os = require('node:os');
const path = require('node:path');
const { detectGlobalPrefix } = require('./update.cjs');

const MANAGED_LAUNCHER_PATTERN = /\/node_modules\/(?:@[^/]+\/)?justmux\/bin\/justmux\.cjs$/;

function safeRealpath(filePath, fs = nativeFs) {
  try {
    return fs.realpathSync(filePath);
  } catch {
    return null;
  }
}

function userPathDirs(pathEnv) {
  if (typeof pathEnv !== 'string' || !pathEnv) return [];
  const dirs = [];
  const seen = new Set();
  for (const entry of pathEnv.split(path.delimiter)) {
    if (!entry || !path.isAbsolute(entry)) continue;
    const normalized = path.resolve(entry);
    const posix = normalized.split(path.sep).join('/');
    if (posix.endsWith('/node_modules/.bin') || posix.endsWith('/node-gyp-bin')) continue;
    if (seen.has(normalized)) continue;
    seen.add(normalized);
    dirs.push(normalized);
  }
  return dirs;
}

function defaultAllowedDirs(env = process.env, homedir = os.homedir) {
  const home = env?.HOME || (typeof homedir === 'function' ? homedir() : '');
  const candidates = [
    '/usr/local/bin',
    '/opt/homebrew/bin',
    home ? path.join(home, '.local', 'bin') : '',
    home ? path.join(home, 'bin') : '',
  ];
  const dirs = [];
  const seen = new Set();
  for (const entry of candidates) {
    if (!entry || !path.isAbsolute(entry)) continue;
    const normalized = path.resolve(entry);
    if (seen.has(normalized)) continue;
    seen.add(normalized);
    dirs.push(normalized);
  }
  return dirs;
}

function classifySlot(filePath, { launcherPath, prefixBin, cmd, fs = nativeFs } = {}) {
  let stat;
  try {
    stat = fs.lstatSync(filePath);
  } catch (error) {
    if (error && error.code === 'ENOENT') return 'missing';
    return 'foreign';
  }
  if (!stat.isSymbolicLink()) return 'foreign';

  let rawTarget;
  try {
    rawTarget = fs.readlinkSync(filePath);
  } catch {
    return 'foreign';
  }

  const resolvedTarget = path.resolve(path.dirname(filePath), rawTarget);
  const resolvedPosix = resolvedTarget.split(path.sep).join('/');
  if (resolvedTarget === launcherPath
    || (prefixBin && cmd && resolvedTarget === path.join(prefixBin, cmd))
    || MANAGED_LAUNCHER_PATTERN.test(resolvedPosix)) {
    return 'managed';
  }

  const realTarget = safeRealpath(filePath, fs);
  if (realTarget) {
    const realPosix = realTarget.split(path.sep).join('/');
    if (realTarget === launcherPath || MANAGED_LAUNCHER_PATTERN.test(realPosix)) {
      return 'managed';
    }
  }

  return 'foreign';
}

function linkAtomically(target, linkPath, fs = nativeFs) {
  try {
    const stat = fs.lstatSync(linkPath);
    if (stat.isSymbolicLink() && fs.readlinkSync(linkPath) === target) {
      return false;
    }
  } catch (error) {
    if (!error || error.code !== 'ENOENT') throw error;
  }

  const tmp = `${linkPath}.justmux-${process.pid}-${Math.random().toString(16).slice(2, 8)}.tmp`;
  fs.symlinkSync(target, tmp);
  try {
    fs.renameSync(tmp, linkPath);
  } catch (error) {
    try { fs.unlinkSync(tmp); } catch { /* Best-effort temporary symlink cleanup. */ }
    throw error;
  }
  return true;
}

function resolveCommandInPath(cmd, pathDirs, fs = nativeFs) {
  for (const dir of pathDirs) {
    const candidate = path.join(dir, cmd);
    try {
      const stat = fs.statSync(candidate);
      if (!stat.isFile()) continue;
      fs.accessSync(candidate, fs.constants.X_OK);
      return { dir, candidate, realpath: safeRealpath(candidate, fs) };
    } catch {
      continue;
    }
  }
  return null;
}

/**
 * Ensure global `jm` and `justmux` links exist in the active npm prefix and
 * remain reachable on PATH without ever overwriting foreign commands.
 */
function ensureGlobalLinks(options = {}) {
  const {
    runtime = process,
    fs = nativeFs,
    packageRoot = path.resolve(__dirname, '..'),
    manifest: providedManifest,
    allowedDirs,
    isGlobal,
    homedir = os.homedir,
  } = options;
  const env = runtime.env ?? {};

  if (env.JUSTMUX_NO_POSTINSTALL_LINK === '1') {
    return { action: 'skip', reason: 'opt-out' };
  }
  if (runtime.platform !== 'darwin' && runtime.platform !== 'linux') {
    return { action: 'skip', reason: 'unsupported-platform' };
  }

  let manifest = providedManifest;
  if (!manifest) {
    try {
      manifest = JSON.parse(fs.readFileSync(path.join(packageRoot, 'package.json'), 'utf8'));
    } catch {
      return { action: 'skip', reason: 'invalid-manifest' };
    }
  }
  if (!manifest || typeof manifest.name !== 'string' || !manifest.name
    || !manifest.bin || typeof manifest.bin !== 'object') {
    return { action: 'skip', reason: 'invalid-manifest' };
  }

  let prefix = null;
  try {
    prefix = detectGlobalPrefix(packageRoot, manifest.name, fs);
  } catch {
    prefix = null;
  }
  const globalFlag = isGlobal === true || env.npm_config_global === 'true' || env.npm_config_global === '1';
  if (!prefix || !globalFlag) {
    return { action: 'skip', reason: 'not-global' };
  }

  const realPackageRoot = safeRealpath(packageRoot, fs);
  if (!realPackageRoot) {
    return { action: 'skip', reason: 'missing-package-root' };
  }
  const launcherPath = path.join(realPackageRoot, 'bin', 'justmux.cjs');
  try {
    if (!fs.statSync(launcherPath).isFile()) {
      return { action: 'skip', reason: 'missing-launcher' };
    }
  } catch {
    return { action: 'skip', reason: 'missing-launcher' };
  }
  try {
    fs.accessSync(launcherPath, fs.constants.X_OK);
  } catch {
    try { fs.chmodSync(launcherPath, 0o755); } catch { /* Best-effort mode repair. */ }
  }

  const commands = Object.keys(manifest.bin).filter((cmd) => (
    cmd && !cmd.includes('/') && !cmd.includes('\\') && cmd !== '.' && cmd !== '..'
  ));
  if (commands.length === 0) {
    return { action: 'skip', reason: 'no-commands' };
  }

  // Step A: repair <prefix>/bin links (covers npm bin-links=false).
  const prefixBin = path.join(prefix, 'bin');
  const prefixLinked = [];
  for (const cmd of commands) {
    const prefixLink = path.join(prefixBin, cmd);
    const slot = classifySlot(prefixLink, { launcherPath, prefixBin, cmd, fs });
    if (slot !== 'missing' && slot !== 'managed') continue;
    try {
      fs.mkdirSync(prefixBin, { recursive: true });
      const target = path.relative(prefixBin, launcherPath);
      if (linkAtomically(target, prefixLink, fs)) {
        prefixLinked.push({ cmd, path: prefixLink, target });
      }
    } catch {
      // Ignore permission errors in <prefix>/bin; Step B may still link into PATH.
    }
  }

  // Step B: verify reachability on the user's real PATH and fall back to a
  // writable allowlisted directory already on PATH when missing.
  const pathDirs = userPathDirs(env.PATH);
  const canonicalPathDirs = new Set();
  for (const dir of pathDirs) {
    canonicalPathDirs.add(path.resolve(dir));
    const realDir = safeRealpath(dir, fs);
    if (realDir) canonicalPathDirs.add(realDir);
  }

  const writableAllowedPathDirs = [];
  const seenAllowed = new Set();
  for (const rawDir of (allowedDirs ?? defaultAllowedDirs(env, homedir))) {
    if (typeof rawDir !== 'string' || !path.isAbsolute(rawDir)) continue;
    const resolvedDir = path.resolve(rawDir);
    const realDir = safeRealpath(resolvedDir, fs);
    if (!canonicalPathDirs.has(resolvedDir) && (!realDir || !canonicalPathDirs.has(realDir))) {
      continue;
    }
    const canonicalKey = realDir || resolvedDir;
    if (seenAllowed.has(canonicalKey)) continue;
    seenAllowed.add(canonicalKey);
    try {
      if (!fs.statSync(resolvedDir).isDirectory()) continue;
      fs.accessSync(resolvedDir, fs.constants.W_OK | fs.constants.X_OK);
      writableAllowedPathDirs.push(resolvedDir);
    } catch {
      continue;
    }
  }

  const linked = [];
  const skipped = [];
  for (const cmd of commands) {
    const hit = resolveCommandInPath(cmd, pathDirs, fs);
    if (hit && hit.realpath === launcherPath) {
      continue;
    }
    if (hit) {
      const slot = classifySlot(hit.candidate, { launcherPath, prefixBin, cmd, fs });
      if (slot === 'managed') {
        try {
          if (linkAtomically(launcherPath, hit.candidate, fs)) {
            linked.push({ cmd, path: hit.candidate, target: launcherPath });
          }
        } catch {
          skipped.push({ cmd, reason: 'link-failed', path: hit.candidate });
        }
      } else {
        skipped.push({ cmd, reason: 'foreign-command', path: hit.candidate });
      }
      continue;
    }

    let created = false;
    for (const dir of writableAllowedPathDirs) {
      const dest = path.join(dir, cmd);
      const slot = classifySlot(dest, { launcherPath, prefixBin, cmd, fs });
      if (slot !== 'missing' && slot !== 'managed') continue;
      try {
        if (linkAtomically(launcherPath, dest, fs)) {
          linked.push({ cmd, path: dest, target: launcherPath });
        }
        created = true;
        break;
      } catch {
        continue;
      }
    }
    if (!created) {
      skipped.push({ cmd, reason: 'no-writable-path-dir' });
    }
  }

  return {
    action: linked.length > 0 || prefixLinked.length > 0 ? 'linked' : 'already-on-path',
    prefix,
    prefixLinked,
    linked,
    skipped,
  };
}

if (require.main === module) {
  try {
    ensureGlobalLinks();
  } catch {
    // Postinstall link repair must never fail npm installation.
  }
}

module.exports = {
  ensureGlobalLinks,
  userPathDirs,
  classifySlot,
  defaultAllowedDirs,
  resolveCommandInPath,
};
