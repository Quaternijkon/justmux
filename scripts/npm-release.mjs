#!/usr/bin/env node
/** Local builds for npm distribution. No command in this file publishes. */
import { spawnSync } from 'node:child_process';
import { mkdirSync, readFileSync, unlinkSync, writeFileSync } from 'node:fs';
import { createRequire } from 'node:module';
import path from 'node:path';
import { fileURLToPath } from 'node:url';
import { compareVersions } from './npm-packages.mjs';

const ROOT = path.resolve(path.dirname(fileURLToPath(import.meta.url)), '..');
const require = createRequire(import.meta.url);
const { SUPPORTED_PLATFORMS } = require('../npm/justmux/lib/launcher.cjs');
const PACKAGER = path.join(ROOT, 'scripts/npm-packages.mjs');
const VERSION_PATTERN = /^(0|[1-9]\d*)\.(0|[1-9]\d*)\.(0|[1-9]\d*)(?:-[0-9A-Za-z-]+(?:\.[0-9A-Za-z-]+)*)?$/;

function help() {
  console.log(`JustMux local npm release preparation (does not publish)

Usage:
  node scripts/npm-release.mjs build [--all | --platform NAME[,NAME...]] [--out DIR] [--dry-run]
  node scripts/npm-release.mjs version VERSION [--dry-run]

Platforms: ${SUPPORTED_PLATFORMS.map(id).join(', ')}

build defaults to the current platform. --all prepares and packs all five npm
packages. A partial build prepares only selected native packages plus the main
package, and cannot be published. Partial and failed builds retain an
.build-incomplete marker that prevents stale artifacts from being published.
Build output defaults to dist/npm.

macOS can build both macOS targets with its Apple SDK. Linux cross-builds use
Zig and cargo-zigbuild. Native Linux musl builds require musl-gcc. This script
does not install tools or alter Git, npm authentication, or published packages.

Examples:
  node scripts/npm-release.mjs build --all
  node scripts/npm-release.mjs build --platform darwin-arm64 --dry-run
  node scripts/npm-release.mjs version 0.1.1
  node scripts/npm-packages.mjs publish --dir dist/npm --dry-run`);
}

function id(platform) {
  return `${platform.platform}-${platform.arch}`;
}

function parseArguments(argv) {
  const [command, ...args] = argv;
  if (!command || command === '--help' || command === '-h') return { help: true };
  if (!['build', 'version'].includes(command)) throw new Error(`Unknown command: ${command}`);
  const options = { command, dryRun: false, all: false, out: 'dist/npm' };
  for (let i = 0; i < args.length; i += 1) {
    const argument = args[i];
    if (argument === '--dry-run') options.dryRun = true;
    else if (argument === '--help' || argument === '-h') options.help = true;
    else if (argument === '--all' && command === 'build') options.all = true;
    else if (['--platform', '--out'].includes(argument) && command === 'build') {
      const value = args[++i];
      if (!value || value.startsWith('--')) throw new Error(`Missing value for ${argument}`);
      if (argument === '--platform') {
        if (options.platforms) throw new Error('--platform can only be supplied once; separate values with commas');
        options.platforms = value.split(',');
      } else options.out = value;
    } else if (command === 'version' && !argument.startsWith('-') && options.version === undefined) {
      options.version = argument;
    } else throw new Error(`Unknown argument: ${argument}`);
  }
  if (options.all && options.platforms) throw new Error('Choose --all or --platform, not both');
  if (command === 'version' && !options.help) {
    const identifiers = options.version?.split('-').slice(1).join('-').split('.') ?? [];
    if (!VERSION_PATTERN.test(options.version ?? '') || identifiers.some((part) => /^0\d+$/.test(part))) {
      throw new Error('Provide a version such as 0.1.1 or 0.2.0-beta.1');
    }
  }
  return options;
}

function run(program, args, { capture = false, timeout = 20 * 60_000 } = {}) {
  const result = spawnSync(program, args, {
    cwd: ROOT,
    encoding: 'utf8',
    stdio: capture ? ['ignore', 'pipe', 'pipe'] : 'inherit',
    timeout,
    shell: false,
  });
  if (result.error) throw new Error(`Could not run ${program}: ${result.error.message}`);
  if (result.status !== 0) {
    const detail = capture ? result.stderr?.trim() : '';
    throw new Error(`${program} ${args.join(' ')} failed (${result.signal ?? result.status})${detail ? `: ${detail}` : ''}`);
  }
  return result.stdout ?? '';
}

function cargoVersion(toml) {
  const section = toml.match(/^\[package\][\s\S]*?(?=^\[|$(?![\s\S]))/m)?.[0];
  return section?.match(/^version\s*=\s*"([^"]+)"/m)?.[1];
}

function selectedPlatforms(options) {
  const requested = options.all ? SUPPORTED_PLATFORMS.map(id)
    : options.platforms ?? [`${process.platform}-${process.arch}`];
  if (new Set(requested).size !== requested.length) throw new Error('Duplicate platform selection');
  return requested.map((requestedId) => {
    const platform = SUPPORTED_PLATFORMS.find((entry) => id(entry) === requestedId);
    if (!platform) throw new Error(`Unsupported platform ${requestedId}; use ${SUPPORTED_PLATFORMS.map(id).join(', ')}`);
    return platform;
  });
}

function buildCommand(platform) {
  if (platform.platform === 'darwin' && process.platform !== 'darwin') {
    throw new Error(`Building ${id(platform)} requires macOS and the Apple SDK; select Linux targets on this host`);
  }
  const nativeLinux = platform.platform === 'linux' && process.platform === 'linux' && platform.arch === process.arch;
  const subcommand = platform.platform === 'linux' && !nativeLinux ? 'zigbuild' : 'build';
  return ['cargo', [subcommand, '--locked', '--release', '--bin', 'jm', '--target', platform.target, '--target-dir', path.join(ROOT, 'target')]];
}

function preflight(platforms, plans) {
  run('cargo', ['--version'], { capture: true, timeout: 15_000 });
  const installed = new Set(run('rustup', ['target', 'list', '--installed'], { capture: true, timeout: 15_000 }).trim().split(/\s+/));
  const missing = platforms.map((entry) => entry.target).filter((target) => !installed.has(target));
  if (missing.length) throw new Error(`Install the required Rust targets first: rustup target add ${missing.join(' ')}`);
  if (plans.some(([, args]) => args[0] === 'zigbuild')) {
    try {
      run('zig', ['version'], { capture: true, timeout: 15_000 });
      // cargo-zigbuild's subcommand does not expose --version; --help checks
      // that Cargo can find and invoke it without starting a compilation.
      run('cargo', ['zigbuild', '--help'], { capture: true, timeout: 15_000 });
    } catch (error) {
      throw new Error(`Linux cross-builds need Zig and cargo-zigbuild. Install Zig (brew install zig on macOS) and run cargo install cargo-zigbuild --locked. ${error.message}`);
    }
  }
  if (platforms.some((entry) => entry.platform === 'darwin')) {
    run('xcrun', ['--show-sdk-path'], { capture: true, timeout: 15_000 });
  }
  if (platforms.some((entry) => entry.platform === 'linux' && process.platform === 'linux' && entry.arch === process.arch)) {
    try {
      run('musl-gcc', ['--version'], { capture: true, timeout: 15_000 });
    } catch {
      throw new Error('Native Linux musl builds require musl-gcc; install musl-tools with your package manager');
    }
  }
}

function build(options) {
  const manifest = JSON.parse(readFileSync(path.join(ROOT, 'npm/justmux/package.json'), 'utf8'));
  const version = cargoVersion(readFileSync(path.join(ROOT, 'Cargo.toml'), 'utf8'));
  if (!version || version !== manifest.version) throw new Error('Cargo.toml and npm/justmux/package.json versions differ; use the version command first');
  const platforms = selectedPlatforms(options);
  const plans = platforms.map(buildCommand);
  const out = path.resolve(ROOT, options.out);
  const complete = platforms.length === SUPPORTED_PLATFORMS.length;
  console.log(`Preparing ${manifest.name}@${version}: ${platforms.map(id).join(', ')}`);
  for (let i = 0; i < platforms.length; i += 1) {
    console.log(`  ${plans[i][0]} ${plans[i][1].join(' ')}`);
  }
  console.log(`Output: ${out}`);
  if (options.dryRun) {
    console.log(complete ? 'Dry run: all packages would be validated and packed; nothing changed.'
      : 'Dry run: partial development build; publication requires all four native packages.');
    return;
  }
  preflight(platforms, plans);
  mkdirSync(out, { recursive: true });
  const incomplete = path.join(out, '.build-incomplete');
  const buildState = `${JSON.stringify({
    version,
    platforms: platforms.map(id),
    complete,
    startedAt: new Date().toISOString(),
  }, null, 2)}\n`;
  writeFileSync(incomplete, buildState);
  for (let i = 0; i < platforms.length; i += 1) {
    const platform = platforms[i];
    run(plans[i][0], plans[i][1]);
    const binary = path.join(ROOT, 'target', platform.target, 'release/jm');
    run(process.execPath, [PACKAGER, 'prepare', '--platform', id(platform), '--binary', binary, '--out', out]);
  }
  run(process.execPath, [PACKAGER, 'prepare', '--main', '--out', out]);
  if (complete) {
    // Only a successful rebuild of every native package may clear this marker.
    // Existing packages from a previous full build cannot make a partial build
    // appear publishable.
    unlinkSync(incomplete);
    try {
      run(process.execPath, [PACKAGER, 'validate', '--dir', out]);
      run(process.execPath, [PACKAGER, 'pack', '--dir', out]);
    } catch (error) {
      writeFileSync(incomplete, buildState);
      throw error;
    }
    const quoted = `'${out.replaceAll("'", "'\\''")}'`;
    console.log(`Local release prepared. Review and test it before publishing:\nnode scripts/npm-packages.mjs publish --dir ${quoted} --dry-run`);
  } else {
    console.log('Partial development build prepared. Publishing is disabled until all four native packages are prepared at the same version.');
  }
}

function version(options) {
  const cargoPath = path.join(ROOT, 'Cargo.toml');
  const lockPath = path.join(ROOT, 'Cargo.lock');
  const npmPath = path.join(ROOT, 'npm/justmux/package.json');
  const toml = readFileSync(cargoPath, 'utf8');
  const lock = readFileSync(lockPath, 'utf8');
  const manifest = JSON.parse(readFileSync(npmPath, 'utf8'));
  const current = cargoVersion(toml);
  if (!current || current !== manifest.version) throw new Error('Source versions already disagree; reconcile Cargo.toml and the npm manifest before changing the version');
  if (compareVersions(options.version, current) <= 0) {
    throw new Error(`The new version must be greater than ${current}; received ${options.version}`);
  }
  let lockMatches = 0;
  const nextLock = lock.replace(/(\[\[package\]\]\r?\nname = "justmux"\r?\nversion = ")[^"]+("\r?\n)/g, (_match, before, after) => {
    lockMatches += 1;
    return `${before}${options.version}${after}`;
  });
  if (lockMatches !== 1) throw new Error('Cargo.lock must contain exactly one local justmux entry');
  const nextToml = toml.replace(/(^\[package\][\s\S]*?^version\s*=\s*")[^"]+("[ \t]*$)/m,
    (_match, before, after) => `${before}${options.version}${after}`);
  if (cargoVersion(nextToml) !== options.version) throw new Error('Could not locate the Cargo package version');
  manifest.version = options.version;
  for (const dependency of Object.keys(manifest.optionalDependencies ?? {})) {
    manifest.optionalDependencies[dependency] = options.version;
  }
  console.log(`${manifest.name}: ${current} -> ${options.version}`);
  if (options.dryRun) {
    console.log('Dry run: Cargo.toml, Cargo.lock, and the npm manifest would be updated; nothing changed.');
    return;
  }
  writeFileSync(cargoPath, nextToml);
  writeFileSync(lockPath, nextLock);
  writeFileSync(npmPath, `${JSON.stringify(manifest, null, 2)}\n`);
  console.log('Version updated. Rebuild every platform before packing or publishing.');
}

try {
  const options = parseArguments(process.argv.slice(2));
  if (options.help) help();
  else if (options.command === 'build') build(options);
  else version(options);
} catch (error) {
  console.error(`npm-release: ${error.message}`);
  process.exitCode = 1;
}
