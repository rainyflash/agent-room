import type { ClientContentEncryption } from '../domain/content-encryption';
import type { PreparedMessageBody, ProtectedMessageBody } from '../domain/publication';

const encoder = new TextEncoder();

// 与 Rust 适配器共用协议：域分隔符、UUID 原始字节、网络字节序长度和 UTF-8 内容。
function associatedData(contextId: string, roomId: string, mediaType: string, size: number) {
  const room = encoder.encode(roomId);
  const media = encoder.encode(mediaType);
  const uuid = Uint8Array.from(contextId.replaceAll('-', '').match(/../gu) ?? [], (pair) =>
    Number.parseInt(pair, 16),
  );
  if (uuid.length !== 16) throw new Error('正文加密上下文无效');
  const number = (value: number) => {
    const bytes = new Uint8Array(8);
    new DataView(bytes.buffer).setBigUint64(0, BigInt(value));
    return bytes;
  };
  const parts = [
    encoder.encode('agent-room:message-content:aad:v1\0'),
    uuid,
    number(room.length),
    room,
    number(media.length),
    media,
    number(size),
  ];
  const result = new Uint8Array(parts.reduce((length, part) => length + part.length, 0));
  let offset = 0;
  for (const part of parts) {
    result.set(part, offset);
    offset += part.length;
  }
  return result;
}

function encode(bytes: Uint8Array): string {
  return btoa(String.fromCharCode(...bytes))
    .replaceAll('+', '-')
    .replaceAll('/', '_')
    .replaceAll('=', '');
}

function decode(value: string): Uint8Array<ArrayBuffer> {
  return Uint8Array.from(atob(value.replaceAll('-', '+').replaceAll('_', '/')), (character) =>
    character.charCodeAt(0),
  );
}

export async function encryptContent(
  body: PreparedMessageBody,
  contextId: string,
  roomId: string,
  mediaType: string,
): Promise<ProtectedMessageBody> {
  const rawKey = crypto.getRandomValues(new Uint8Array(32));
  const nonce = crypto.getRandomValues(new Uint8Array(12));
  const key = await crypto.subtle.importKey('raw', rawKey, 'AES-GCM', false, ['encrypt']);
  const bytes = new Uint8Array(
    await crypto.subtle.encrypt(
      {
        name: 'AES-GCM',
        iv: nonce,
        additionalData: associatedData(contextId, roomId, mediaType, body.bytes.length),
      },
      key,
      body.bytes,
    ),
  );
  const digestSha256 = [...new Uint8Array(await crypto.subtle.digest('SHA-256', bytes))]
    .map((byte) => byte.toString(16).padStart(2, '0'))
    .join('');
  return {
    body: { bytes, digestSha256 },
    encryption: {
      algorithm: 'io.github.rainyflash.agentroom.content.aes-256-gcm.v1',
      contextId,
      keyBase64Url: encode(rawKey),
      nonceBase64Url: encode(nonce),
      plaintextSizeBytes: body.bytes.length,
    },
  };
}

export async function decryptContent(
  bytes: Uint8Array,
  encryption: ClientContentEncryption,
  roomId: string,
  mediaType: string,
): Promise<Uint8Array<ArrayBuffer>> {
  if (bytes.length !== encryption.plaintextSizeBytes + 16) throw new Error('密文长度无效');
  const key = await crypto.subtle.importKey(
    'raw',
    decode(encryption.keyBase64Url),
    'AES-GCM',
    false,
    ['decrypt'],
  );
  return new Uint8Array(
    await crypto.subtle.decrypt(
      {
        name: 'AES-GCM',
        iv: decode(encryption.nonceBase64Url),
        additionalData: associatedData(
          encryption.contextId,
          roomId,
          mediaType,
          encryption.plaintextSizeBytes,
        ),
      },
      key,
      Uint8Array.from(bytes),
    ),
  );
}
