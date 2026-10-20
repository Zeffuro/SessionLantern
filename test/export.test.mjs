import assert from 'node:assert/strict';
import { execFileSync } from 'node:child_process';
import { fileURLToPath } from 'node:url';
import test, { before } from 'node:test';
import { decryptSyntheticExport } from '../scripts/verify-export.mjs';

const root = fileURLToPath(new URL('../', import.meta.url));
const PASSWORD = '  synthetic café 🔑 fixture  ';
let fixture;

before(() => {
  fixture = execFileSync('cargo', [
    'run', '--quiet', '--manifest-path', 'src-tauri/Cargo.toml', '--example', 'export-fixture',
  ], { cwd: root, timeout: 300_000, maxBuffer: 128 * 1024, windowsHide: true });
});

test('Rust export decrypts with WebCrypto and the documented schema', async () => {
  const output = fixture;
  const envelope = JSON.parse(output);
  assert.equal(envelope.kdf.iterations, 600_000);
  assert.equal(Buffer.from(envelope.kdf.salt, 'base64').length, 16);
  assert.equal(Buffer.from(envelope.cipher.iv, 'base64').length, 12);
  for (const secret of ['fixture-session-value', 'fixture-region-value', 'fixture-token-value',
    'fixture-fingerprint-value', PASSWORD, 'sessionId']) {
    assert.equal(output.includes(secret), false);
  }
  const payload = await decryptSyntheticExport(output, PASSWORD);
  assert.equal(payload.service, 'tiktok');
  assert.ok(payload.exportedAt <= Date.now());
  assert.deepEqual(payload.cookies, {
    sessionId: 'fixture-session-value', targetIdc: 'fixture-region-value',
    msToken: 'fixture-token-value', verifyFp: 'fixture-fingerprint-value',
  });
});

test('WebCrypto rejects a wrong or trimmed password and altered ciphertext', async () => {
  const output = fixture;
  await assert.rejects(decryptSyntheticExport(output, 'wrong synthetic password'));
  await assert.rejects(decryptSyntheticExport(output, PASSWORD.trim()));
  const envelope = JSON.parse(output);
  const ciphertext = Buffer.from(envelope.ciphertext, 'base64');
  ciphertext[0] ^= 1;
  envelope.ciphertext = ciphertext.toString('base64');
  await assert.rejects(decryptSyntheticExport(JSON.stringify(envelope), PASSWORD));
});

test('Verifier rejects unsupported metadata and oversized files before derivation', async () => {
  for (const change of [
    envelope => { envelope.version = 2; },
    envelope => { envelope.kdf.iterations = 600_001; },
    envelope => { envelope.kdf.iterations = 599_999; },
    envelope => { envelope.cipher.iv = Buffer.alloc(11).toString('base64'); },
    envelope => { envelope.kdf.salt = Buffer.alloc(15).toString('base64'); },
    envelope => { envelope.cipher.name = 'AES-CBC'; },
  ]) {
    const envelope = JSON.parse(fixture);
    change(envelope);
    await assert.rejects(decryptSyntheticExport(JSON.stringify(envelope), PASSWORD));
  }
  await assert.rejects(decryptSyntheticExport(Buffer.alloc(65_537), PASSWORD));
  await assert.rejects(decryptSyntheticExport(Buffer.from([0xff]), PASSWORD));
});
