import { webcrypto } from 'node:crypto';

const decoder = new TextDecoder('utf-8', { fatal: true });
const encoder = new TextEncoder();
const ITERATIONS = 600_000;
const MAX_FILE_BYTES = 64 * 1024;

function exactKeys(value, keys) {
  return value !== null && typeof value === 'object' && !Array.isArray(value)
    && Object.keys(value).sort().join(',') === [...keys].sort().join(',');
}

function base64(value, size) {
  if (typeof value !== 'string' || !/^(?:[A-Za-z0-9+/]{4})*(?:[A-Za-z0-9+/]{2}==|[A-Za-z0-9+/]{3}=)?$/.test(value)) {
    throw new Error('Invalid encrypted export.');
  }
  const bytes = Buffer.from(value, 'base64');
  if (bytes.toString('base64') !== value || (size !== undefined && bytes.length !== size)) {
    throw new Error('Invalid encrypted export.');
  }
  return bytes;
}

export async function decryptSyntheticExport(file, password) {
  let plaintext;
  try {
    const bytes = typeof file === 'string' ? encoder.encode(file) : new Uint8Array(file);
    if (bytes.byteLength > MAX_FILE_BYTES) throw new Error();
    const envelope = JSON.parse(decoder.decode(bytes));
    if (!exactKeys(envelope, ['format', 'version', 'kdf', 'cipher', 'ciphertext'])
      || envelope.format !== 'session-lantern' || envelope.version !== 1
      || !exactKeys(envelope.kdf, ['name', 'iterations', 'salt'])
      || envelope.kdf.name !== 'PBKDF2-SHA256' || envelope.kdf.iterations !== ITERATIONS
      || !exactKeys(envelope.cipher, ['name', 'iv']) || envelope.cipher.name !== 'AES-256-GCM') {
      throw new Error();
    }
    const salt = base64(envelope.kdf.salt, 16);
    const iv = base64(envelope.cipher.iv, 12);
    const ciphertext = base64(envelope.ciphertext);
    if (ciphertext.byteLength < 16 || typeof password !== 'string'
      || Array.from(password).length < 12 || encoder.encode(password).length > 1024) {
      throw new Error();
    }
    const passwordBytes = encoder.encode(password);
    let material;
    try {
      material = await webcrypto.subtle.importKey('raw', passwordBytes, 'PBKDF2', false, ['deriveKey']);
    } finally {
      passwordBytes.fill(0);
    }
    const key = await webcrypto.subtle.deriveKey({
      name: 'PBKDF2', hash: 'SHA-256', salt, iterations: ITERATIONS,
    }, material, { name: 'AES-GCM', length: 256 }, false, ['decrypt']);
    plaintext = new Uint8Array(await webcrypto.subtle.decrypt({
      name: 'AES-GCM', iv, additionalData: encoder.encode('session-lantern:v1'), tagLength: 128,
    }, key, ciphertext));
    const payload = JSON.parse(decoder.decode(plaintext));
    if (!exactKeys(payload, ['service', 'exportedAt', 'cookies']) || payload.service !== 'tiktok'
      || !Number.isSafeInteger(payload.exportedAt) || payload.exportedAt <= 0
      || !exactKeys(payload.cookies, ['sessionId', 'targetIdc', 'msToken', 'verifyFp'])
      || Object.values(payload.cookies).some(value => typeof value !== 'string' || value.length === 0)) {
      throw new Error();
    }
    return payload;
  } catch {
    throw new Error('Invalid encrypted export or password.');
  } finally {
    plaintext?.fill(0);
  }
}
