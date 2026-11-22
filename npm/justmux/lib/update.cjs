'use strict';

const nativeFs = require('node:fs');
const os = require('node:os');
const path = require('node:path');
const { spawn } = require('node:child_process');
const { updatePrompt } = require('./update-prompt.cjs');

const DAY = 24 * 60 * 60 * 1000;
const BODY_LIMIT = 64 * 1024;
const CONTINUE = Object.freeze({ action: 'continue' });
const VERSION = /^(0|[1-9]\d*)\.(0|[1-9]\d*)\.(0|[1-9]\d*)(?:-([0-9A-Za-z-]+(?:\.[0-9A-Za-z-]+)*))?(?:\+([0-9A-Za-z-]+(?:\.[0-9A-Za-z-]+)*))?$/;
const PACKAGE = /^(?:@[a-z0-9][a-z0-9._-]*\/)?[a-z0-9][a-z0-9._-]*$/;

function versionParts(version) {
  const match = typeof version === 'string' && VERSION.exec(version);
  if (!match) throw new Error('Invalid semantic version');
  const prerelease = match[4]?.split('.') ?? [];
  if (prerelease.some((part) => /^0\d+$/.test(part))) throw new Error('Invalid semantic version');
  return { core: match.slice(1, 4).map(BigInt), prerelease };
}

function compareVersions(left, right) {
  const a = versionParts(left);
  const b = versionParts(right);
  for (let i = 0; i < 3; i++) {
    if (a.core[i] !== b.core[i]) return a.core[i] < b.core[i] ? -1 : 1;
  }
  if (!a.prerelease.length || !b.prerelease.length) {
    return Number(!a.prerelease.length) - Number(!b.prerelease.length);
  }
  for (let i = 0; i < Math.max(a.prerelease.length, b.prerelease.length); i++) {
    const x = a.prerelease[i];
    const y = b.prerelease[i];
    if (x === y) continue;
    if (x === undefined) return -1;
    if (y === undefined) return 1;
    const numericX = /^\d+$/.test(x);
    const numericY = /^\d+$/.test(y);
    if (numericX !== numericY) return numericX ? -1 : 1;
    return (numericX ? BigInt(x) < BigInt(y) : x < y) ? -1 : 1;
  }
  return 0;
}

// Resolve links first: `npm link` to a source checkout is not an installation
// this updater should replace. Local node_modules and npx caches are excluded.
function detectGlobalPrefix(packageRoot, name, fs = nativeFs) {
  if (!PACKAGE.test(name ?? '')) return null;
  const actual = fs.realpathSync(packageRoot);
  let directory = actual;
  for (const segment of name.split('/').reverse()) {
    if (path.basename(directory) !== segment) return null;
    directory = path.dirname(directory);
  }
  if (path.basename(directory) !== 'node_modules' || path.basename(path.dirname(directory)) !== 'lib') {
    return null;
  }
  return path.dirname(path.dirname(directory));
}

function defaultCachePath(runtime = process) {
  const env = runtime.env ?? {};
  if (env.XDG_CACHE_HOME && path.isAbsolute(env.XDG_CACHE_HOME)) {
    return path.join(env.XDG_CACHE_HOME, 'justmux', 'update.json');
  }
  const home = env.HOME || os.homedir();
  return path.join(home, runtime.platform === 'darwin' ? 'Library/Caches' : '.cache', 'justmux', 'update.json');
}

async function fetchLatest(name, { fetchImpl = globalThis.fetch, timeoutMs = 3000, maxTimeoutMs = 3000 } = {}) {
  if (!PACKAGE.test(name ?? '') || typeof fetchImpl !== 'function') throw new Error('Invalid update request');
  const cap = Number.isFinite(maxTimeoutMs) && maxTimeoutMs > 0 ? maxTimeoutMs : 3000;
  const timeout = Number.isFinite(timeoutMs) && timeoutMs > 0 ? Math.min(timeoutMs, cap) : cap;
  const controller = new AbortController();
  let timer;
  const deadline = new Promise((_, reject) => {
    timer = setTimeout(() => {
      controller.abort();
      reject(new Error('Update check timed out'));
    }, timeout);
  });
  try {
    return await Promise.race([deadline, (async () => {
      const response = await fetchImpl(`https://registry.npmjs.org/${encodeURIComponent(name)}/latest`, {
        method: 'GET', redirect: 'error', signal: controller.signal,
        headers: { Accept: 'application/json' },
      });
      if (!response.ok) {
        await response.body?.cancel();
        throw new Error('Update metadata unavailable');
      }
      if (Number(response.headers.get('content-length')) > BODY_LIMIT) {
        await response.body?.cancel();
        throw new Error('Update metadata too large');
      }
      const reader = response.body?.getReader();
      if (!reader) throw new Error('Missing update metadata');
      const chunks = [];
      let length = 0;
      try {
        while (true) {
          const { done, value } = await reader.read();
          if (done) break;
          length += value.byteLength;
          if (length > BODY_LIMIT) {
            await reader.cancel();
            throw new Error('Update metadata too large');
          }
          chunks.push(Buffer.from(value));
        }
      } finally { reader.releaseLock(); }
      const metadata = JSON.parse(Buffer.concat(chunks, length).toString('utf8'));
      if (metadata.name !== name) throw new Error('Update package name mismatch');
      versionParts(metadata.version);
      return metadata.version;
    })()]);
  } finally {
    clearTimeout(timer);
    controller.abort();
  }
}

function nativeInstall(executable, args, options) {
  return new Promise((resolve) => {
    const child = spawn(executable, args, options);
    child.once('error', () => resolve({ status: 1 }));
    child.once('close', (status, signal) => resolve({ status, signal }));
  });
}

const quote = (value) => `'${value.replaceAll("'", "'\\''")}'`;

/** An optional startup step; errors must never stop the native application. */
async function maybeUpdate(args, options = {}) {
  const runtime = options.runtime ?? process;
  const env = runtime.env ?? {};
  if (!Array.isArray(args) || !(args.length === 0 || (args.length === 1 && (args[0] === '--fullscreen' || args[0] === '--inline')))
    || !runtime.stdin?.isTTY || !runtime.stdout?.isTTY || env.CI
    || env.JUSTMUX_NO_UPDATE_CHECK === '1' || env.NO_UPDATE_CHECK === '1'
    || !((runtime.platform === 'darwin' && runtime.arch === 'arm64') || (runtime.platform === 'linux' && runtime.arch === 'x64'))) {
    return CONTINUE;
  }
  try {
    const fs = options.fs ?? nativeFs;
    const packageRoot = options.packageRoot ?? path.resolve(__dirname, '..');
    const manifest = options.manifest ?? JSON.parse(fs.readFileSync(path.join(packageRoot, 'package.json'), 'utf8'));
    versionParts(manifest.version);
    const prefix = detectGlobalPrefix(packageRoot, manifest.name, fs);
    if (!prefix) return CONTINUE;
    const cachePath = options.cachePath ?? defaultCachePath(runtime);
    const now = (options.now ?? Date.now)();
    if (!Number.isFinite(now) || now < 0) return CONTINUE;
    let cache;
    let dismissedVersion;
    try {
      cache = JSON.parse(fs.readFileSync(cachePath, 'utf8'));
      if (typeof cache.dismissedVersion === 'string') {
        try {
          versionParts(cache.dismissedVersion);
          dismissedVersion = cache.dismissedVersion;
        } catch { /* Ignore invalid saved dismissals. */ }
      }
      versionParts(cache.version);
      if (!Number.isFinite(cache.time) || cache.time < 0 || cache.time > now || now - cache.time >= DAY) cache = undefined;
    } catch { cache = undefined; }
    const save = () => {
      try {
        fs.mkdirSync(path.dirname(cachePath), { recursive: true });
        fs.writeFileSync(cachePath, `${JSON.stringify(cache)}\n`, { mode: 0o600 });
      } catch { /* A read-only cache must not prevent starting JustMux. */ }
    };
    if (!cache) {
      cache = { version: manifest.version, time: now, ...(dismissedVersion && { dismissedVersion }) };
      // Persist attempts as well as successes: offline launches do not repeat
      // a network timeout on every invocation throughout the same day.
      save();
      try {
        cache.version = await fetchLatest(manifest.name, options);
        save();
      } catch { return CONTINUE; }
    }
    if (compareVersions(cache.version, manifest.version) <= 0 || cache.dismissedVersion === cache.version
      || (!versionParts(manifest.version).prerelease.length && versionParts(cache.version).prerelease.length)) return CONTINUE;
    const answer = await (options.prompt ?? updatePrompt)(
      { currentVersion: manifest.version, latestVersion: cache.version }, runtime,
    );
    if (answer?.action === 'exit') return answer;
    if (answer === 'skip-version') {
      cache.dismissedVersion = cache.version;
      save();
    }
    if (answer !== 'update') return CONTINUE;
    const installArgs = ['install', '--global', '--prefix', prefix, `${manifest.name}@${cache.version}`, '--include=optional'];
    let result;
    try {
      result = await (options.install ?? nativeInstall)('npm', installArgs, { stdio: 'inherit', shell: false });
    } catch { result = { status: 1 }; }
    if (result?.status === 0 && !result.signal) {
      runtime.stdout.write('JustMux updated. Run jm (or justmux) again to start the new version.\n');
      return { action: 'exit', code: 0 };
    }
    runtime.stdout.write(`JustMux update failed. Continuing with the installed version.\nTo retry: npm ${installArgs.map(quote).join(' ')}\n`);
    return CONTINUE;
  } catch { return CONTINUE; }
}

function isUpdateCommand(args) {
  return Array.isArray(args) && (args[0] === 'update' || args[0] === 'upgrade');
}

function readDismissedVersion(fs, cachePath) {
  try {
    const cache = JSON.parse(fs.readFileSync(cachePath, 'utf8'));
    if (typeof cache?.dismissedVersion === 'string') {
      versionParts(cache.dismissedVersion);
      return cache.dismissedVersion;
    }
  } catch { /* Ignore invalid or missing cache state. */ }
  return undefined;
}

function writeUpdateCache(fs, cachePath, entry) {
  try {
    fs.mkdirSync(path.dirname(cachePath), { recursive: true });
    fs.writeFileSync(cachePath, `${JSON.stringify(entry)}\n`, { mode: 0o600 });
  } catch { /* A read-only cache must not fail an explicit update. */ }
}

const UPDATE_USAGE = [
  'Usage: jm update [--check]',
  '',
  'Check npm for a newer JustMux release and update the global installation.',
  'Aliases: jm upgrade, justmux update, justmux upgrade',
  '',
  'Options:',
  '  --check     Only check whether a newer release is available',
  '  -h, --help  Print help',
  '',
].join('\n');

/** Explicit `jm update` / `justmux update` workflow bypassing passive opt-outs and cache. */
async function runUpdate(restArgs = [], options = {}) {
  const runtime = options.runtime ?? process;
  const out = runtime.stdout ?? process.stdout;
  const err = runtime.stderr ?? runtime.stdout ?? process.stderr;
  let checkOnly = false;
  for (const arg of Array.isArray(restArgs) ? restArgs : []) {
    if (arg === '-h' || arg === '--help') {
      out.write(UPDATE_USAGE);
      return { action: 'exit', code: 0 };
    }
    if (arg === '--check') {
      checkOnly = true;
      continue;
    }
    err.write(`justmux: unexpected argument '${arg}' for 'update' (try 'jm update --help')\n`);
    return { action: 'exit', code: 2 };
  }

  const fs = options.fs ?? nativeFs;
  const packageRoot = options.packageRoot ?? path.resolve(__dirname, '..');
  let manifest;
  try {
    manifest = options.manifest ?? JSON.parse(fs.readFileSync(path.join(packageRoot, 'package.json'), 'utf8'));
    if (!PACKAGE.test(manifest?.name ?? '')) throw new Error('Invalid package name');
    versionParts(manifest.version);
  } catch (error) {
    err.write(`justmux: invalid package metadata (${error.message}).\n`);
    return { action: 'exit', code: 1 };
  }

  const supported = (runtime.platform === 'darwin' && runtime.arch === 'arm64')
    || (runtime.platform === 'linux' && runtime.arch === 'x64');
  if (!checkOnly && !supported) {
    err.write(
      `justmux: unsupported platform ${runtime.platform}/${runtime.arch}. JustMux supports macOS arm64 and Linux x64.\n`
      + `Run: npm install -g ${manifest.name}@latest\n`,
    );
    return { action: 'exit', code: 1 };
  }

  const prefix = detectGlobalPrefix(packageRoot, manifest.name, fs);
  if (!prefix && !checkOnly) {
    err.write(
      'justmux: cannot self-update a non-global installation (such as npx, local node_modules, or npm link).\n'
      + `Run: npm install -g ${manifest.name}@latest\n`,
    );
    return { action: 'exit', code: 1 };
  }

  let latest;
  try {
    latest = await fetchLatest(manifest.name, {
      ...options,
      timeoutMs: options.timeoutMs ?? 10000,
      maxTimeoutMs: options.maxTimeoutMs ?? 15000,
    });
  } catch (error) {
    err.write(
      `justmux: failed to check npm for updates (${error.message}).\n`
      + `To update manually: npm install -g ${manifest.name}@latest\n`,
    );
    return { action: 'exit', code: 1 };
  }

  if (!versionParts(manifest.version).prerelease.length && versionParts(latest).prerelease.length) {
    out.write(`JustMux ${manifest.version} is already up to date.\n`);
    return { action: 'exit', code: 0 };
  }

  const cachePath = options.cachePath ?? defaultCachePath(runtime);
  const now = (options.now ?? Date.now)();
  const validNow = Number.isFinite(now) && now >= 0;
  const refreshCache = (dismissedVersion) => {
    if (validNow) {
      writeUpdateCache(fs, cachePath, { version: latest, time: now, ...(dismissedVersion && { dismissedVersion }) });
    }
  };

  if (compareVersions(latest, manifest.version) <= 0) {
    refreshCache(readDismissedVersion(fs, cachePath));
    out.write(`JustMux ${manifest.version} is already up to date.\n`);
    return { action: 'exit', code: 0 };
  }

  if (checkOnly) {
    refreshCache(readDismissedVersion(fs, cachePath));
    out.write(
      `JustMux update available: ${manifest.version} -> ${latest}\n`
      + "Run 'jm update' (or 'justmux update') to install.\n",
    );
    return { action: 'exit', code: 0 };
  }

  out.write(`Updating JustMux ${manifest.version} -> ${latest}...\n`);
  const installArgs = ['install', '--global', '--prefix', prefix, `${manifest.name}@${latest}`, '--include=optional'];
  let result;
  try {
    result = await (options.install ?? nativeInstall)('npm', installArgs, { stdio: 'inherit', shell: false });
  } catch {
    result = { status: 1 };
  }
  if (result?.status === 0 && !result.signal) {
    refreshCache(undefined);
    out.write(`JustMux updated to ${latest}. Run jm (or justmux) again to start the new version.\n`);
    return { action: 'exit', code: 0 };
  }
  err.write(`JustMux update failed.\nTo retry: npm ${installArgs.map(quote).join(' ')}\n`);
  return { action: 'exit', code: result?.status || 1 };
}

module.exports = {
  maybeUpdate,
  isUpdateCommand,
  runUpdate,
  compareVersions,
  detectGlobalPrefix,
  defaultCachePath,
  fetchLatest,
  updatePrompt,
};
