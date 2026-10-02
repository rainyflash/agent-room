import { z } from 'zod';

import { controlPlaneEndpoint } from '@/shared/http/control-plane-endpoint';

import {
  CrossSigningUploadFailure,
  type EncryptionKeyEscrow,
  type EscrowedEncryptionKey,
} from './matrix-device-signing';

const KEY_BYTES = 32;
/** 控制面说“这个账户还没有钥匙”时的错误码。 */
const MISSING_KEY = 'account.encryption_key_missing';

const keySchema = z
  .object({
    schemaVersion: z.literal(1),
    keyId: z.string().min(1).max(255),
    key: z.string().regex(/^[A-Za-z0-9+/]+={0,2}$/u),
  })
  .strict();

/** 控制面替账户保管的密钥存储钥匙（ADR 0011）。只经本人的登录会话（Cookie）存取。 */
export class ControlPlaneEncryptionKeyEscrow implements EncryptionKeyEscrow {
  constructor(
    private readonly options: { readonly baseUrl: string; readonly fetch: typeof globalThis.fetch },
  ) {}

  async fetch(): Promise<EscrowedEncryptionKey | null> {
    const response = await this.#request('/account/encryption-key', { method: 'GET' });
    // 只有控制面明说“这个账户还没有钥匙”才算没有；旧控制面没有这个接口时也是 404，
    // 那时当作失败，免得以为没有钥匙就去重建密钥存储。
    if (response.status === 404 && (await errorCode(response)) === MISSING_KEY) return null;
    if (!response.ok) throw new Error(`取钥匙失败：HTTP ${String(response.status)}`);
    const parsed = keySchema.parse(await response.json());
    const key = decodeBase64(parsed.key);
    if (key.length !== KEY_BYTES) throw new Error('服务器给的钥匙长度不对。');
    return { key, keyId: parsed.keyId };
  }

  async store(key: EscrowedEncryptionKey): Promise<void> {
    const response = await this.#request('/account/encryption-key', {
      body: JSON.stringify({ key: encodeBase64(key.key), keyId: key.keyId }),
      headers: { 'Content-Type': 'application/json' },
      method: 'PUT',
    });
    if (!response.ok) throw new Error(`存钥匙失败：HTTP ${String(response.status)}`);
  }

  async replaceCrossSigningKeys(keys: Readonly<Record<string, unknown>>): Promise<void> {
    let response: Response;
    try {
      response = await this.#request('/account/encryption-reset', {
        body: JSON.stringify(keys),
        headers: { 'Content-Type': 'application/json' },
        method: 'POST',
      });
    } catch {
      throw new CrossSigningUploadFailure(true, '连不上控制面，没能代传新的签名公钥。');
    }
    if (response.ok) return;
    // 公钥不对、来源不对、太勤：再试也一样。控制面或 Synapse 暂时不可用：可以再试。
    const retryable = response.status >= 500;
    throw new CrossSigningUploadFailure(
      retryable,
      `控制面没能代传新的签名公钥：HTTP ${String(response.status)}`,
    );
  }

  async #request(path: string, init: RequestInit): Promise<Response> {
    return await this.options.fetch(controlPlaneEndpoint(this.options.baseUrl, path), {
      ...init,
      cache: 'no-store',
      credentials: 'include',
      signal: AbortSignal.timeout(10_000),
    });
  }
}

function encodeBase64(bytes: Uint8Array): string {
  let binary = '';
  for (const byte of bytes) binary += String.fromCharCode(byte);
  return btoa(binary);
}

function decodeBase64(text: string): Uint8Array {
  const binary = atob(text);
  const bytes = new Uint8Array(binary.length);
  for (let index = 0; index < binary.length; index += 1) bytes[index] = binary.charCodeAt(index);
  return bytes;
}

async function errorCode(response: Response): Promise<string | null> {
  try {
    const body: unknown = await response.json();
    return typeof body === 'object' &&
      body !== null &&
      'code' in body &&
      typeof body.code === 'string'
      ? body.code
      : null;
  } catch {
    return null;
  }
}
