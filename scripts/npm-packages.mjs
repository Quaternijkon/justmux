#!/usr/bin/env node
// Prepare immutable, version-matched npm packages without publishing source templates.
import { createHash } from 'node:crypto';
import { createRequire } from 'node:module';
import { constants } from 'node:fs';
import { access, chmod, copyFile, cp, mkdir, readFile, writeFile } from 'node:fs/promises';
import { basename, dirname, join, resolve } from 'node:path';
import { fileURLToPath, pathToFileURL } from 'node:url';
import { spawnSync } from 'node:child_process';

export const ROOT = resolve(dirname(fileURLToPath(import.meta.url)), '..');
const SOURCE = join(ROOT, 'npm/justmux');
const require = createRequire(import.meta.url);
export const { SUPPORTED_PLATFORMS, platformPackageName } = require('../npm/justmux/lib/launcher.cjs');
const digest = (bytes, algorithm = 'sha256', encoding = 'hex') => createHash(algorithm).update(bytes).digest(encoding);
const json = async (path) => JSON.parse(await readFile(path, 'utf8'));
const save = (path, value) => writeFile(path, `${JSON.stringify(value, null, 2)}\n`);
const VERSION_PATTERN = /^(0|[1-9]\d*)\.(0|[1-9]\d*)\.(0|[1-9]\d*)(?:-([0-9A-Za-z-]+(?:\.[0-9A-Za-z-]+)*))?$/;

function versionParts(version) {
  const match = VERSION_PATTERN.exec(version);
  const prerelease = match?.[4]?.split('.') ?? [];
  if (!match || prerelease.some((part) => /^0\d+$/.test(part))) {
    throw new Error(`Invalid release version: ${version}`);
  }
  return { core: match.slice(1, 4).map(BigInt), prerelease };
}

export function compareVersions(left, right) {
  const a = versionParts(left);
  const b = versionParts(right);
  for (let i = 0; i < a.core.length; i++) {
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

export async function configuration() {
  const manifest = await json(join(SOURCE, 'package.json'));
  const cargo = await readFile(join(ROOT, 'Cargo.toml'), 'utf8');
  const section = cargo.match(/^\[package\]([\s\S]*?)(?=^\[|(?![\s\S]))/m)?.[1];
  const version = section?.match(/^version\s*=\s*"([^"]+)"/m)?.[1];
  if (!version || version !== manifest.version) {
    throw new Error('Cargo.toml and npm/justmux/package.json versions must match.');
  }
  versionParts(version);
  return manifest;
}

export function platformFor(key) {
  const platform = SUPPORTED_PLATFORMS.find((p) => `${p.platform}-${p.arch}` === key);
  if (!platform) throw new Error(`Unsupported release platform: ${key}`);
  return platform;
}

export function inspectBinary(bytes, platform) {
  if (bytes.length < 64) throw new Error('The native executable is too small.');
  if (platform.platform === 'darwin') {
    const cpu = platform.arch === 'arm64' ? 0x0100000c : 0x01000007;
    if (bytes.readUInt32LE(0) !== 0xfeedfacf || bytes.readUInt32LE(4) !== cpu) {
      throw new Error(`Expected a Mach-O executable for ${platform.platform}/${platform.arch}.`);
    }
  } else {
    const machine = platform.arch === 'arm64' ? 183 : 62;
    if (bytes.subarray(0, 4).toString('hex') !== '7f454c46'
      || bytes[4] !== 2 || bytes[5] !== 1 || bytes.readUInt16LE(18) !== machine) {
      throw new Error(`Expected a 64-bit Linux ELF executable for ${platform.arch}.`);
    }
    const offset = Number(bytes.readBigUInt64LE(32));
    const size = bytes.readUInt16LE(54);
    const count = bytes.readUInt16LE(56);
    if (size < 56 || count === 0 || !Number.isSafeInteger(offset) || offset + size * count > bytes.length) {
      throw new Error('Invalid ELF program-header table.');
    }
    for (let i = 0; i < count; i++) {
      if (bytes.readUInt32LE(offset + i * size) === 3) {
        throw new Error('Linux npm packages require static musl binaries without an external ELF interpreter.');
      }
    }
  }
}

function command(program, args, options = {}) {
  const result = spawnSync(program, args, {
    cwd: ROOT, encoding: 'utf8', timeout: 120_000, maxBuffer: 8 * 1024 * 1024, ...options,
  });
  if (result.error) throw result.error;
  if (result.status !== 0) {
    const detail = result.stderr || result.stdout || `exit ${result.status ?? result.signal}`;
    throw new Error(`${program} ${args[0]} failed:\n${detail}`);
  }
  return result.stdout;
}

export async function prepare({ out, platform, binary, main = false }) {
  const source = await configuration();
  const destination = join(resolve(out), main ? 'justmux' : `justmux-${platform}`);
  await mkdir(destination, { recursive: true });
  const shared = {
    version: source.version, license: source.license, repository: source.repository,
    homepage: source.homepage, bugs: source.bugs,
    publishConfig: { access: 'public', registry: 'https://registry.npmjs.org/' },
  };
  if (main) {
    const { private: ignoredPrivate, scripts: ignoredScripts, ...manifest } = source;
    void ignoredPrivate;
    void ignoredScripts;
    manifest.optionalDependencies = Object.fromEntries(SUPPORTED_PLATFORMS.map((p) => [
      platformPackageName(source.name, p.platform, p.arch), source.version,
    ]));
    await cp(join(SOURCE, 'bin'), join(destination, 'bin'), { recursive: true });
    await cp(join(SOURCE, 'lib'), join(destination, 'lib'), { recursive: true });
    await copyFile(join(SOURCE, 'README.md'), join(destination, 'README.md'));
    await chmod(join(destination, 'bin/justmux.cjs'), 0o755);
    await save(join(destination, 'package.json'), { ...manifest, ...shared });
  } else {
    const spec = platformFor(platform);
    if (!binary) throw new Error('--binary is required for native platform packages.');
    const bytes = await readFile(resolve(binary));
    inspectBinary(bytes, spec);
    await mkdir(join(destination, 'bin'), { recursive: true });
    await copyFile(resolve(binary), join(destination, 'bin/jm'));
    await chmod(join(destination, 'bin/jm'), 0o755);
    await save(join(destination, 'package.json'), {
      name: platformPackageName(source.name, spec.platform, spec.arch), ...shared,
      description: `JustMux native executable for ${spec.platform}/${spec.arch}.`,
      os: [spec.platform], cpu: [spec.arch], files: ['bin/jm', 'LICENSE', 'README.md'],
      justmux: { target: spec.target, sha256: digest(bytes) },
    });
    await writeFile(join(destination, 'README.md'),
      `# ${source.name} (${platform})\n\nPlatform binary. Install \`${source.name}\` to get both \`jm\` and \`justmux\`.\n`);
  }
  await copyFile(join(ROOT, 'LICENSE'), join(destination, 'LICENSE'));
  return destination;
}

export async function validate(directory) {
  let incomplete = false;
  try {
    await access(join(directory, '.build-incomplete'));
    incomplete = true;
  } catch (error) {
    if (error.code !== 'ENOENT') throw error;
  }
  if (incomplete) {
    throw new Error('This distribution is an incomplete or partial build. Run npm-release.mjs build --all successfully before packing or publishing.');
  }
  const source = await configuration();
  const manifests = [];
  for (const spec of SUPPORTED_PLATFORMS) {
    const key = `${spec.platform}-${spec.arch}`;
    const path = join(directory, `justmux-${key}`);
    const manifest = await json(join(path, 'package.json'));
    if (manifest.name !== platformPackageName(source.name, spec.platform, spec.arch)
      || manifest.version !== source.version || manifest.private || manifest.scripts
      || manifest.os?.join() !== spec.platform || manifest.cpu?.join() !== spec.arch
      || manifest.justmux?.target !== spec.target) {
      throw new Error(`Inconsistent platform package ${key}.`);
    }
    const bytes = await readFile(join(path, 'bin/jm'));
    inspectBinary(bytes, spec);
    if (manifest.justmux.sha256 !== digest(bytes)) throw new Error(`Native binary checksum mismatch: ${key}`);
    await access(join(path, 'bin/jm'), constants.X_OK);
    if (spec.platform === process.platform && spec.arch === process.arch) {
      const output = command(join(path, 'bin/jm'), ['--version']);
      if (output.trim() !== `jm ${source.version}`) throw new Error(`Native version mismatch: ${output.trim()}`);
    }
    manifests.push({ path, manifest, platform: key });
  }
  const path = join(directory, 'justmux');
  const manifest = await json(join(path, 'package.json'));
  const expected = Object.fromEntries(manifests.map((p) => [p.manifest.name, source.version]));
  if (manifest.name !== source.name || manifest.version !== source.version || manifest.private || manifest.scripts
    || manifest.bin?.jm !== 'bin/justmux.cjs' || manifest.bin?.justmux !== 'bin/justmux.cjs'
    || JSON.stringify(manifest.optionalDependencies) !== JSON.stringify(expected)) {
    throw new Error('The main npm package must reference every exact-version native package and expose both commands.');
  }
  await access(join(path, 'bin/justmux.cjs'), constants.X_OK);
  manifests.push({ path, manifest, platform: null });
  return manifests;
}

export async function pack(directory) {
  const packages = await validate(directory);
  const tarballs = join(directory, 'tarballs');
  await mkdir(tarballs, { recursive: true });
  const records = [];
  for (const { path, manifest, platform } of packages) {
    const result = JSON.parse(command('npm', ['pack', path, '--ignore-scripts', '--json', '--pack-destination', tarballs]));
    const filename = result[0]?.filename;
    if (!filename || basename(filename) !== filename) throw new Error('npm pack returned an invalid filename.');
    const bytes = await readFile(join(tarballs, filename));
    records.push({ name: manifest.name, version: manifest.version, platform, filename,
      sha256: digest(bytes), integrity: `sha512-${digest(bytes, 'sha512', 'base64')}` });
  }
  await save(join(directory, 'npm-release.json'), {
    version: packages.at(-1).manifest.version, main: packages.at(-1).manifest.name, packages: records,
  });
  await writeFile(join(tarballs, 'SHA256SUMS'), records.map((r) => `${r.sha256}  ${r.filename}\n`).join(''));
  return records;
}

export async function verifyPacked(directory) {
  const packages = await validate(directory);
  const release = await json(join(directory, 'npm-release.json'));
  const source = packages.at(-1).manifest;
  if (release.main !== source.name || release.version !== source.version || release.packages?.length !== packages.length) {
    throw new Error('Packed release does not match the current package configuration. Run pack again.');
  }
  for (let i = 0; i < packages.length; i++) {
    const record = release.packages[i];
    const expected = packages[i].manifest;
    if (record.name !== expected.name || record.version !== expected.version
      || typeof record.filename !== 'string' || basename(record.filename) !== record.filename) {
      throw new Error('Invalid packed package name, version, or publish order.');
    }
    const path = join(directory, 'tarballs', record.filename);
    const bytes = await readFile(path);
    if (record.sha256 !== digest(bytes) || record.integrity !== `sha512-${digest(bytes, 'sha512', 'base64')}`) {
      throw new Error(`Packed tarball checksum mismatch: ${record.filename}`);
    }
    const metadata = JSON.parse(command('tar', ['-xOf', path, 'package/package.json']));
    if (JSON.stringify(metadata) !== JSON.stringify(expected)) throw new Error(`Packed metadata changed: ${record.name}`);
    const contents = packages[i].platform
      ? ['bin/jm'] : ['bin/justmux.cjs', 'lib/launcher.cjs'];
    for (const file of contents) {
      const packedBytes = command('tar', ['-xOf', path, `package/${file}`], { encoding: null });
      const preparedBytes = await readFile(join(packages[i].path, file));
      if (!packedBytes.equals(preparedBytes)) throw new Error(`Packed file changed: ${record.name}/${file}`);
    }
  }
  return release;
}

/** Wait for anonymous registry readers to see the exact, downloadable packages.
 * npm can accept a publication before either its metadata or tarball is public.
 * Successful records leave the pending set, so binaries are downloaded once.
 */
export async function waitForPublished(records, options = {}) {
  const {
    timeoutMs = 10 * 60_000,
    intervalMs = 15_000,
    requestTimeoutMs = 15_000,
    fetchImpl = globalThis.fetch,
    now = Date.now,
    sleep = (milliseconds) => new Promise((done) => setTimeout(done, milliseconds)),
    log = console.log,
  } = options;
  if (![timeoutMs, intervalMs, requestTimeoutMs].every((value) => Number.isFinite(value) && value > 0)) {
    throw new Error('Registry wait timeouts and intervals must be positive finite milliseconds.');
  }
  const pending = new Map(records.map((record) => [`${record.name}@${record.version}`, record]));
  const deadline = now() + timeoutMs;
  const timedOut = () => new Error(
    `Timed out waiting for public npm availability: ${[...pending.keys()].join(', ')}. `
    + 'npm may still be processing these packages. Run verify-registry again; do not republish them.',
  );
  const request = async (url, label) => {
    const remaining = deadline - now();
    if (remaining <= 0) throw timedOut();
    let response;
    try {
      response = await fetchImpl(url, {
        method: 'GET', headers: { Accept: 'application/json', 'Cache-Control': 'no-cache' },
        redirect: 'error', signal: AbortSignal.timeout(Math.max(1, Math.floor(Math.min(requestTimeoutMs, remaining)))),
      });
    } catch (error) {
      // Do not echo request headers, environment configuration, or credentials.
      throw new Error(`Public npm request failed for ${label} (${error.name ?? 'network error'}).`, { cause: error });
    }
    if (response.status === 404) {
      await response.body?.cancel();
      return undefined;
    }
    if (!response.ok) {
      await response.body?.cancel();
      throw new Error(`The public npm registry returned HTTP ${response.status} for ${label}.`);
    }
    return response;
  };
  while (pending.size) {
    if (now() >= deadline) throw timedOut();
    const visible = await Promise.all([...pending.entries()].map(async ([label, record]) => {
      const url = `https://registry.npmjs.org/${encodeURIComponent(record.name)}/${encodeURIComponent(record.version)}`;
      const response = await request(url, label);
      if (!response) return undefined;
      let metadata;
      try {
        metadata = await response.json();
      } catch (error) {
        throw new Error(`The public npm registry returned invalid metadata for ${label}.`, { cause: error });
      }
      if (metadata.name !== record.name || metadata.version !== record.version
        || metadata.dist?.integrity !== record.integrity) {
        throw new Error(`Public npm integrity or package identity mismatch for ${label}. Do not republish this version.`);
      }
      let tarball;
      try {
        tarball = new URL(metadata.dist.tarball);
      } catch {
        throw new Error(`The public npm registry did not provide a valid tarball URL for ${label}.`);
      }
      if (tarball.protocol !== 'https:' || tarball.hostname !== 'registry.npmjs.org'
        || tarball.username || tarball.password) {
        throw new Error(`The public npm registry returned an unexpected tarball origin for ${label}.`);
      }
      const archive = await request(tarball.href, `${label}'s tarball`);
      if (!archive) return undefined;
      let bytes;
      try {
        bytes = Buffer.from(await archive.arrayBuffer());
      } catch (error) {
        throw new Error(`Could not download the public npm tarball for ${label}.`, { cause: error });
      }
      if (`sha512-${digest(bytes, 'sha512', 'base64')}` !== record.integrity) {
        throw new Error(`Public npm tarball integrity mismatch for ${label}. Do not republish this version.`);
      }
      return label;
    }));
    for (const label of visible) {
      if (label !== undefined) {
        pending.delete(label);
        log(`Publicly available with verified integrity: ${label}`);
      }
    }
    if (!pending.size) break;
    const remaining = deadline - now();
    if (remaining <= 0) throw timedOut();
    log(`Waiting for npm processing: ${[...pending.keys()].join(', ')}`);
    await sleep(Math.min(intervalMs, remaining));
  }
  return records;
}

export async function publish(directory, { dryRun = false, provenance = false } = {}) {
  const release = await verifyPacked(directory);
  const tag = release.version.includes('-') ? 'next' : 'latest';
  const trustedPublisher = provenance && process.env.GITHUB_ACTIONS === 'true' && process.env.ACTIONS_ID_TOKEN_REQUEST_URL;
  if (!dryRun && !trustedPublisher) {
    command('npm', ['whoami', '--registry=https://registry.npmjs.org/', '--fetch-retries=0', '--fetch-timeout=15000']);
  }
  if (!dryRun) {
    assertTagCanAdvance(release, tag);
  }
  for (const record of release.packages) {
    if (!dryRun && record.name === release.main) {
      // Never expose the main package until every optional native dependency
      // can actually be fetched by an unauthenticated npm install.
      await waitForPublished(release.packages.filter((entry) => entry.name !== release.main));
      assertTagCanAdvance(release, tag);
    }
    const path = join(directory, 'tarballs', record.filename);
    if (!dryRun) {
      const existing = registryValue(`${record.name}@${record.version}`, 'dist.integrity');
      if (existing !== undefined) {
        if (existing !== record.integrity) {
          throw new Error(`${record.name}@${record.version} already exists with different contents. Bump the version.`);
        }
        console.log(`Already published with matching integrity: ${record.name}@${record.version}`);
        continue;
      }
    }
    const args = ['publish', path, '--access=public', '--ignore-scripts',
      '--registry=https://registry.npmjs.org/', '--tag', tag];
    if (dryRun) args.push('--dry-run');
    if (provenance) args.push('--provenance');
    console.log(`${dryRun ? 'Dry run' : 'Publishing'}: ${record.name}@${record.version}`);
    // Browser-based 2FA may take several minutes. Keep the publishing process
    // alive while the account owner completes npm's official verification.
    command('npm', args, { stdio: 'inherit', timeout: 10 * 60_000 });
  }
  if (!dryRun) {
    await waitForPublished(release.packages.filter((record) => record.name === release.main));
    console.log(`${release.main}@${release.version} is publicly downloadable with every native dependency verified.`);
  }
}

function registryValue(packageName, field) {
  const result = spawnSync('npm', ['view', packageName, field, '--json',
    '--registry=https://registry.npmjs.org/', '--fetch-retries=0', '--fetch-timeout=15000'],
  { encoding: 'utf8', timeout: 30_000 });
  if (result.error) throw result.error;
  if (result.status === 0) return result.stdout.trim() ? JSON.parse(result.stdout) : undefined;
  if (`${result.stderr}\n${result.stdout}`.includes('E404')) return undefined;
  throw new Error(`Could not check the registry for ${packageName}: ${result.stderr}`);
}

function assertTagCanAdvance(release, tag) {
  const current = registryValue(release.main, `dist-tags.${tag}`);
  if (current && compareVersions(release.version, current) < 0) {
    throw new Error(`Refusing to move ${release.main}'s ${tag} tag backwards from ${current} to ${release.version}. Bump the version.`);
  }
}

export async function main(argv = process.argv.slice(2)) {
  const action = argv.shift();
  const options = {};
  for (let i = 0; i < argv.length; i++) {
    const key = argv[i];
    if (['--main', '--dry-run', '--provenance'].includes(key)) options[key.slice(2)] = true;
    else if (['--platform', '--binary', '--out', '--dir'].includes(key) && argv[i + 1]) options[key.slice(2)] = argv[++i];
    else throw new Error(`Unknown or incomplete argument: ${key}`);
  }
  const directory = resolve(options.dir || options.out || join(ROOT, 'dist/npm'));
  if (action === 'prepare') {
    console.log(`Prepared ${await prepare({ out: directory, platform: options.platform, binary: options.binary, main: options.main })}`);
  } else if (action === 'validate') {
    console.log(`Validated ${(await validate(directory)).length} packages.`);
  } else if (action === 'pack') {
    console.log(`Packed ${(await pack(directory)).length} packages into ${join(directory, 'tarballs')}`);
  } else if (action === 'publish') {
    await publish(directory, { dryRun: options['dry-run'], provenance: options.provenance });
  } else if (action === 'verify-registry') {
    const release = await verifyPacked(directory);
    await waitForPublished(release.packages);
    console.log(`${release.main}@${release.version} and every native package are publicly downloadable with verified integrity.`);
  } else throw new Error('Usage: npm-packages.mjs prepare|validate|pack|publish|verify-registry [--main|--platform PLATFORM --binary FILE] [--out DIR|--dir DIR] [--dry-run] [--provenance]');
}

if (process.argv[1] && import.meta.url === pathToFileURL(resolve(process.argv[1])).href) {
  main().catch((error) => { console.error(`npm-packages: ${error.message}`); process.exitCode = 1; });
}
