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

async function fetchLatest(name, { fetchImpl = globalThis.fetch, timeoutMs = 3000 } = {}) {
  if (!PACKAGE.test(name ?? '') || typeof fetchImpl !== 'function') throw new Error('Invalid update request');
  const timeout = Number.isFinite(timeoutMs) && timeoutMs > 0 ? Math.min(timeoutMs, 3000) : 3000;
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
      runtime.stdout.write('JustMux updated. Run jm again to start the new version.\n');
      return { action: 'exit', code: 0 };
    }
    runtime.stdout.write(`JustMux update failed. Continuing with the installed version.\nTo retry: npm ${installArgs.map(quote).join(' ')}\n`);
    return CONTINUE;
  } catch { return CONTINUE; }
}

module.exports = { maybeUpdate, compareVersions, detectGlobalPrefix, defaultCachePath, fetchLatest, updatePrompt };
