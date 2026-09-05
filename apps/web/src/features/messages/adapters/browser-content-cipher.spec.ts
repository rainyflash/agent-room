import { describe, expect, it } from 'vitest';
import { decryptContent, encryptContent } from './browser-content-cipher';
import type { ClientContentEncryption } from '../domain/content-encryption';

const room = '!private:matrix.test';
const metadata: ClientContentEncryption = {
  algorithm: 'io.github.rainyflash.agentroom.content.aes-256-gcm.v1',
  contextId: '01990d9e-8400-7000-8000-000000000003',
  keyBase64Url: 'BwcHBwcHBwcHBwcHBwcHBwcHBwcHBwcHBwcHBwcHBwc',
  nonceBase64Url: 'CQkJCQkJCQkJCQkJ',
  plaintextSizeBytes: 14,
};
const vector = Uint8Array.from(
  'c33824711b4d2edd2c23ac5d889e13f199b74796f6b1bc4c77a5f69755eb'.match(/../gu) ?? [],
  (pair) => Number.parseInt(pair, 16),
);
describe('Web 与 Rust 正文加密协议', () => {
  it('读取独立 AES-GCM 实现生成的固定协议向量', async () => {
    expect(
      new TextDecoder().decode(await decryptContent(vector, metadata, room, 'text/plain')),
    ).toBe('你好，Agent');
  });
  it('篡改房间、媒体类型、上下文、字节和密钥均拒绝解密', async () => {
    for (const changed of [
      { bytes: vector, encryption: metadata, room: '!other:matrix.test', media: 'text/plain' },
      { bytes: vector, encryption: metadata, room, media: 'text/markdown' },
      {
        bytes: vector,
        encryption: { ...metadata, contextId: '01990d9e-8400-7000-8000-000000000004' },
        room,
        media: 'text/plain',
      },
      {
        bytes: Uint8Array.from(vector, (byte, index) => (index === 0 ? byte ^ 1 : byte)),
        encryption: metadata,
        room,
        media: 'text/plain',
      },
      {
        bytes: vector,
        encryption: { ...metadata, keyBase64Url: 'A'.repeat(43) },
        room,
        media: 'text/plain',
      },
    ])
      await expect(
        decryptContent(changed.bytes, changed.encryption, changed.room, changed.media),
      ).rejects.toThrow();
  });
  it('加密输出为密文并保留完整 Unicode 文本', async () => {
    const body = { bytes: new TextEncoder().encode('你好，Agent'), digestSha256: 'a'.repeat(64) };
    const encrypted = await encryptContent(body, metadata.contextId, room, 'text/plain');
    expect(encrypted.body.bytes).not.toEqual(body.bytes);
    expect(encrypted.body.bytes.length).toBe(body.bytes.length + 16);
    expect(encrypted.encryption).toBeDefined();
    if (encrypted.encryption === undefined) throw new Error('缺少加密元数据');
    expect(
      await decryptContent(encrypted.body.bytes, encrypted.encryption, room, 'text/plain'),
    ).toEqual(body.bytes);
  });
});
