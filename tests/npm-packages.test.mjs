import assert from 'node:assert/strict';
import { createHash } from 'node:crypto';
import { mkdtemp, readFile, rm, writeFile } from 'node:fs/promises';
import { tmpdir } from 'node:os';
import { join } from 'node:path';
import test from 'node:test';
import {
  compareVersions, configuration, inspectBinary, pack, platformFor, prepare, publish, validate, waitForPublished,
} from '../scripts/npm-packages.mjs';

function registryFixture(name = 'justmux-linux-x64') {
  const bytes = Buffer.from(`native tarball for ${name}`);
  const integrity = `sha512-${createHash('sha512').update(bytes).digest('base64')}`;
  const record = { name, version: '0.1.0', integrity };
  const metadata = { name, version: '0.1.0', dist: {
    integrity, tarball: `https://registry.npmjs.org/${name}/-/${name}-0.1.0.tgz`,
  } };
  return { bytes, record, metadata };
}

function fakeClock() {
  let elapsed = 0;
  const sleeps = [];
  return { now: () => elapsed, sleeps, sleep: async (milliseconds) => {
    sleeps.push(milliseconds);
    elapsed += milliseconds;
  } };
}

test('public verification waits through metadata and tarball propagation without redownloading successful packages', async () => {
  const first = registryFixture('justmux-linux-x64');
  const second = registryFixture('justmux-darwin-arm64');
  const clock = fakeClock();
  const requests = new Map();
  const messages = [];
  await waitForPublished([first.record, second.record], {
    ...clock, timeoutMs: 100, intervalMs: 10, log: (message) => messages.push(message),
    fetchImpl: async (url) => {
      const count = (requests.get(url) ?? 0) + 1;
      requests.set(url, count);
      const fixture = url.includes('arm64') ? second : first;
      if (url.endsWith('.tgz')) {
        // Metadata may be public before the tarball itself can be downloaded.
        if (fixture === second && count === 1) return new Response('', { status: 404 });
        return new Response(fixture.bytes);
      }
      if (fixture === second && count === 1) return new Response('', { status: 404 });
      return Response.json(fixture.metadata);
    },
  });
  assert.deepEqual(clock.sleeps, [10, 10]);
  assert.equal(requests.get('https://registry.npmjs.org/justmux-linux-x64/0.1.0'), 1);
  assert.equal(requests.get(first.metadata.dist.tarball), 1);
  assert.equal(requests.get('https://registry.npmjs.org/justmux-darwin-arm64/0.1.0'), 3);
  assert.equal(requests.get(second.metadata.dist.tarball), 2);
  assert.equal(messages.filter((message) => message.startsWith('Publicly available')).length, 2);
});

test('public verification rejects metadata or downloaded tarball integrity mismatches immediately', async () => {
  const fixture = registryFixture();
  const clock = fakeClock();
  await assert.rejects(() => waitForPublished([fixture.record], {
    ...clock, log() {}, fetchImpl: async () => Response.json({
      ...fixture.metadata, dist: { ...fixture.metadata.dist, integrity: 'sha512-different' },
    }),
  }), /integrity or package identity mismatch/);
  assert.deepEqual(clock.sleeps, []);
  await assert.rejects(() => waitForPublished([fixture.record], {
    ...clock, log() {}, fetchImpl: async (url) => url.endsWith('.tgz')
      ? new Response('wrong tarball bytes') : Response.json(fixture.metadata),
  }), /tarball integrity mismatch/);
  assert.deepEqual(clock.sleeps, []);
});

test('public verification has an overall deadline and suggests verification instead of republishing', async () => {
  const fixture = registryFixture();
  const clock = fakeClock();
  let requests = 0;
  await assert.rejects(() => waitForPublished([fixture.record], {
    ...clock, timeoutMs: 25, intervalMs: 10, log() {}, fetchImpl: async () => {
      requests += 1;
      return new Response('', { status: 404 });
    },
  }), /Timed out.*justmux-linux-x64@0\.1\.0.*verify-registry again; do not republish/);
  assert.deepEqual(clock.sleeps, [10, 10, 5]);
  assert.equal(requests, 3);
});

test('public verification reports server and network failures without retrying or leaking details', async () => {
  const fixture = registryFixture();
  const clock = fakeClock();
  await assert.rejects(() => waitForPublished([fixture.record], {
    ...clock, log() {}, fetchImpl: async () => new Response('', { status: 503 }),
  }), /HTTP 503/);
  await assert.rejects(() => waitForPublished([fixture.record], {
    ...clock, log() {}, fetchImpl: async () => { throw new Error('hidden credential text'); },
  }), (error) => /Public npm request failed/.test(error.message) && !error.message.includes('hidden credential'));
  assert.deepEqual(clock.sleeps, []);
});

test('release versions advance in semantic order, including numeric prereleases', () => {
  const ordered = ['0.1.0', '0.2.0-alpha', '0.2.0-alpha.2', '0.2.0-alpha.10', '0.2.0-beta', '0.2.0', '0.10.0', '1.0.0'];
  for (let i = 1; i < ordered.length; i++) {
    assert.equal(compareVersions(ordered[i], ordered[i - 1]), 1);
    assert.equal(compareVersions(ordered[i - 1], ordered[i]), -1);
  }
  assert.equal(compareVersions('0.1.0', '0.1.0'), 0);
  assert.equal(compareVersions('0.2.0-beta.2', '0.2.0-beta.2'), 0);
  assert.throws(() => compareVersions('01.0.0', '1.0.0'), /Invalid release version/);
  assert.throws(() => compareVersions('1.0.0-beta.01', '1.0.0'), /Invalid release version/);
});

test('partial or interrupted builds cannot be validated, packed, or published', async (t) => {
  const directory = await mkdtemp(join(tmpdir(), 'jm-incomplete-'));
  t.after(() => rm(directory, { recursive: true, force: true }));
  await writeFile(join(directory, '.build-incomplete'), 'partial build');
  await assert.rejects(() => validate(directory), /incomplete or partial build/);
  await assert.rejects(() => pack(directory), /incomplete or partial build/);
  await assert.rejects(() => publish(directory, { dryRun: true }), /incomplete or partial build/);
});

function macho(arch) {
  const bytes = Buffer.alloc(64);
  bytes.writeUInt32LE(0xfeedfacf);
  bytes.writeUInt32LE(arch === 'arm64' ? 0x0100000c : 0x01000007, 4);
  return bytes;
}

function elf(arch, interpreter = false) {
  const bytes = Buffer.alloc(120);
  bytes.write('7f454c46', 'hex');
  bytes[4] = 2;
  bytes[5] = 1;
  bytes.writeUInt16LE(arch === 'arm64' ? 183 : 62, 18);
  bytes.writeBigUInt64LE(64n, 32);
  bytes.writeUInt16LE(56, 54);
  bytes.writeUInt16LE(1, 56);
  bytes.writeUInt32LE(interpreter ? 3 : 1, 64);
  return bytes;
}

test('release validation rejects wrong CPU, malformed binaries, and dynamic Linux loaders', () => {
  assert.doesNotThrow(() => inspectBinary(macho('arm64'), platformFor('darwin-arm64')));
  assert.doesNotThrow(() => inspectBinary(elf('x64'), platformFor('linux-x64')));
  assert.throws(() => inspectBinary(elf('x64', true), platformFor('linux-x64')), /static musl/);
  assert.throws(() => inspectBinary(macho('x64'), platformFor('darwin-arm64')), /Mach-O/);
  assert.throws(() => inspectBinary(elf('arm64'), platformFor('linux-x64')), /ELF/);
  assert.throws(() => inspectBinary(Buffer.alloc(12), platformFor('linux-x64')), /too small/);
  const badHeaders = elf('x64');
  badHeaders.writeBigUInt64LE(99999n, 32);
  assert.throws(() => inspectBinary(badHeaders, platformFor('linux-x64')), /header/);
  assert.throws(() => platformFor('win32-x64'), /Unsupported/);
});

test('prepared main is publishable while its source remains protected and every dependency is exact', async (t) => {
  const directory = await mkdtemp(join(tmpdir(), 'jm-pack-main-'));
  t.after(() => rm(directory, { recursive: true, force: true }));
  const source = await configuration();
  assert.equal(source.private, true);
  const prepared = await prepare({ out: directory, main: true });
  const manifest = JSON.parse(await readFile(join(prepared, 'package.json'), 'utf8'));
  assert.equal(manifest.private, undefined);
  assert.equal(manifest.scripts, undefined);
  assert.equal(manifest.bin.jm, manifest.bin.justmux);
  assert.equal(Object.keys(manifest.optionalDependencies).length, 2);
  assert.ok(Object.values(manifest.optionalDependencies).every((v) => v === source.version));
  assert.equal((await readFile(join(prepared, 'LICENSE'), 'utf8')).includes('MIT License'), true);
  await assert.rejects(() => publish(directory, { dryRun: true }), /ENOENT/);
});

test('native metadata hashes the actual executable, and a changed binary blocks the release', async (t) => {
  const directory = await mkdtemp(join(tmpdir(), 'jm-pack-native-'));
  t.after(() => rm(directory, { recursive: true, force: true }));
  // This first target is checked before any host execution in validate().
  const binary = join(directory, 'fixture');
  const bytes = macho('arm64');
  await writeFile(binary, bytes);
  const prepared = await prepare({ out: directory, platform: 'darwin-arm64', binary });
  const manifest = JSON.parse(await readFile(join(prepared, 'package.json'), 'utf8'));
  assert.equal(manifest.justmux.sha256, createHash('sha256').update(bytes).digest('hex'));
  assert.deepEqual(manifest.os, ['darwin']);
  assert.deepEqual(manifest.cpu, ['arm64']);
  bytes[63] = 42;
  await writeFile(join(prepared, 'bin/jm'), bytes);
  await assert.rejects(() => validate(directory), /checksum mismatch/);
});
