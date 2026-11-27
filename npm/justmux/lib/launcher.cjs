'use strict';

const fs = require('node:fs');
const path = require('node:path');

const SUPPORTED_PLATFORMS = Object.freeze([
  Object.freeze({ platform: 'darwin', arch: 'arm64', target: 'aarch64-apple-darwin' }),
  Object.freeze({ platform: 'linux', arch: 'x64', target: 'x86_64-unknown-linux-musl' }),
]);

function platformPackageName(mainName, platform, arch) {
  if (!SUPPORTED_PLATFORMS.some((entry) => entry.platform === platform && entry.arch === arch)) {
    throw new Error(
      `Unsupported platform ${platform}/${arch}. JustMux supports macOS arm64 and Linux x64.`,
    );
  }
  return `${mainName}-${platform}-${arch}`;
}

function resolveNativeExecutable(manifest, options = {}) {
  const {
    platform = process.platform,
    arch = process.arch,
    resolvePackage = (name) => require.resolve(`${name}/package.json`),
    readFile = fs.readFileSync,
    access = fs.accessSync,
  } = options;
  const packageName = platformPackageName(manifest.name, platform, arch);
  const reinstall = `npm install -g ${manifest.name}@${manifest.version} --include=optional`;
  let manifestPath;
  try {
    manifestPath = resolvePackage(packageName);
  } catch (error) {
    throw new Error(
      `The native package ${packageName}@${manifest.version} is missing. `
      + `Optional dependencies must be enabled. Reinstall with: ${reinstall}`,
      { cause: error },
    );
  }

  let nativeManifest;
  try {
    nativeManifest = JSON.parse(readFile(manifestPath, 'utf8'));
  } catch (error) {
    throw new Error(`Cannot read ${packageName}'s package metadata. Reinstall with: ${reinstall}`, { cause: error });
  }
  if (nativeManifest.name !== packageName || nativeManifest.version !== manifest.version) {
    throw new Error(
      `The native package must be ${packageName}@${manifest.version}; `
      + `found ${nativeManifest.name}@${nativeManifest.version}. Reinstall with: ${reinstall}`,
    );
  }

  const executable = path.join(path.dirname(manifestPath), 'bin', 'jm');
  try {
    access(executable, fs.constants.X_OK);
  } catch (error) {
    throw new Error(`The native executable is missing or not executable. Reinstall with: ${reinstall}`, { cause: error });
  }
  return executable;
}

function run(args, options = {}) {
  const {
    manifest = require('../package.json'),
    runtime = process,
    resolveExecutable = resolveNativeExecutable,
  } = options;
  // Reject unsupported platforms before checking the runtime's exec API.
  platformPackageName(manifest.name, runtime.platform, runtime.arch);
  if (typeof runtime.execve !== 'function') {
    throw new Error(
      'Node.js 22.15.0 or newer with process.execve support is required. '
      + 'Upgrade Node.js, or install the standalone native binary: '
      + 'curl -fsSL https://raw.githubusercontent.com/Quaternijkon/justmux/main/install.sh | sh',
    );
  }
  const executable = resolveExecutable(manifest, { platform: runtime.platform, arch: runtime.arch });
  const entry = typeof runtime.argv?.[1] === 'string' ? path.basename(runtime.argv[1]).toLowerCase() : '';
  const env = entry === 'justmux' || entry === 'jm'
    ? { ...runtime.env, JUSTMUX_INVOKED_AS: entry }
    : runtime.env;
  try {
    // Replace Node rather than supervising tmux: signals, stdio and the PID
    // belong directly to the Rust application, then to tmux after attach.
    return runtime.execve(executable, [executable, ...args], env);
  } catch (error) {
    throw new Error(`Cannot execute the JustMux native binary: ${error.message}`, { cause: error });
  }
}

module.exports = { SUPPORTED_PLATFORMS, platformPackageName, resolveNativeExecutable, run };
