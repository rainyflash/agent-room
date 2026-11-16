import type { MatrixClient, MatrixEvent } from 'matrix-js-sdk';
import { afterEach, describe, expect, it, vi } from 'vitest';

import { MatrixClientRegistry } from '@/shared/matrix/matrix-client-registry';
import {
  CRYPTO_EVENT_DEVICES_UPDATED,
  CRYPTO_EVENT_USER_TRUST_STATUS_CHANGED,
  MATRIX_EVENT_DECRYPTED,
} from '@/shared/matrix/matrix-sdk-enums';

import { MatrixSenderTrustRetry } from './matrix-sender-trust-retry';

const AGENT = '@_agent_new:agent-room.test';
const OTHER = '@_agent_other:agent-room.test';

type Handler = (...args: unknown[]) => void;

class FakeClient {
  readonly crypto = { kind: 'crypto' };
  readonly #handlers = new Map<string, Set<Handler>>();

  on(event: string, handler: Handler): void {
    const handlers = this.#handlers.get(event) ?? new Set<Handler>();
    handlers.add(handler);
    this.#handlers.set(event, handlers);
  }

  off(event: string, handler: Handler): void {
    this.#handlers.get(event)?.delete(handler);
  }

  emit(event: string, ...args: unknown[]): void {
    for (const handler of this.#handlers.get(event) ?? []) handler(...args);
  }

  listeners(event: string): number {
    return this.#handlers.get(event)?.size ?? 0;
  }

  getCrypto(): object {
    return this.crypto;
  }

  asClient(): MatrixClient {
    return this as unknown as MatrixClient;
  }
}

/** 一条解不开的事件；`stillFails` 为真时再解还是失败，SDK 照样发出“解密完了”。 */
function undecryptable(client: FakeClient, code: string, sender = AGENT) {
  const state = { failed: true, stillFails: false };
  const event = {
    attemptDecryption: vi.fn((crypto: unknown, options: unknown) => {
      void crypto;
      void options;
      state.failed = state.stillFails;
      client.emit(MATRIX_EVENT_DECRYPTED, event);
      return Promise.resolve();
    }),
    decryptionFailureReason: code,
    getSender: () => sender,
    isDecryptionFailure: () => state.failed,
  };
  return { event, state, asEvent: () => event as unknown as MatrixEvent };
}

function harness() {
  const client = new FakeClient();
  const registry = new MatrixClientRegistry();
  registry.replace(client.asClient());
  const retry = new MatrixSenderTrustRetry(registry, { fallbackRetryMs: 15_000 });
  return { client, registry, retry };
}

afterEach(() => {
  vi.useRealTimers();
});

describe('MatrixSenderTrustRetry', () => {
  it('发送方的设备列表更新以后，把判成“没签名”的那一条再解一次', () => {
    const { client } = harness();
    const message = undecryptable(client, 'UNSIGNED_SENDER_DEVICE');
    client.emit(MATRIX_EVENT_DECRYPTED, message.asEvent());

    client.emit(CRYPTO_EVENT_DEVICES_UPDATED, [OTHER]);
    expect(message.event.attemptDecryption).not.toHaveBeenCalled();

    client.emit(CRYPTO_EVENT_DEVICES_UPDATED, [AGENT]);
    expect(message.event.attemptDecryption).toHaveBeenCalledExactlyOnceWith(client.crypto, {
      isRetry: true,
    });
    expect(message.state.failed).toBe(false);

    // 解开了就不再记着：之后的更新不再碰它。
    client.emit(CRYPTO_EVENT_DEVICES_UPDATED, [AGENT]);
    expect(message.event.attemptDecryption).toHaveBeenCalledTimes(1);
  });

  it('签名身份更新、设备“不认识”的也一样', () => {
    const { client } = harness();
    const message = undecryptable(client, 'UNKNOWN_SENDER_DEVICE');
    client.emit(MATRIX_EVENT_DECRYPTED, message.asEvent());

    client.emit(CRYPTO_EVENT_USER_TRUST_STATUS_CHANGED, AGENT, {});
    expect(message.event.attemptDecryption).toHaveBeenCalledTimes(1);
  });

  it('再解还是失败的记回来，等下一次更新', () => {
    const { client } = harness();
    const message = undecryptable(client, 'UNSIGNED_SENDER_DEVICE');
    message.state.stillFails = true;
    client.emit(MATRIX_EVENT_DECRYPTED, message.asEvent());

    client.emit(CRYPTO_EVENT_DEVICES_UPDATED, [AGENT]);
    client.emit(CRYPTO_EVENT_DEVICES_UPDATED, [AGENT]);
    expect(message.event.attemptDecryption).toHaveBeenCalledTimes(2);
  });

  it('缺密钥、签名身份换过的不归它管', () => {
    const { client } = harness();
    const missingKey = undecryptable(client, 'MEGOLM_UNKNOWN_INBOUND_SESSION_ID');
    const changed = undecryptable(client, 'SENDER_IDENTITY_PREVIOUSLY_VERIFIED');
    client.emit(MATRIX_EVENT_DECRYPTED, missingKey.asEvent());
    client.emit(MATRIX_EVENT_DECRYPTED, changed.asEvent());

    client.emit(CRYPTO_EVENT_DEVICES_UPDATED, [AGENT]);
    expect(missingKey.event.attemptDecryption).not.toHaveBeenCalled();
    expect(changed.event.attemptDecryption).not.toHaveBeenCalled();
  });

  it('一直等不到更新的，过一会儿自己再解一次，只一次', () => {
    vi.useFakeTimers();
    const { client } = harness();
    const message = undecryptable(client, 'UNSIGNED_SENDER_DEVICE');
    message.state.stillFails = true;
    client.emit(MATRIX_EVENT_DECRYPTED, message.asEvent());

    vi.advanceTimersByTime(14_999);
    expect(message.event.attemptDecryption).not.toHaveBeenCalled();
    vi.advanceTimersByTime(1);
    expect(message.event.attemptDecryption).toHaveBeenCalledTimes(1);

    vi.advanceTimersByTime(60_000);
    expect(message.event.attemptDecryption).toHaveBeenCalledTimes(1);
    // 设备真更新了照样再解。
    client.emit(CRYPTO_EVENT_DEVICES_UPDATED, [AGENT]);
    expect(message.event.attemptDecryption).toHaveBeenCalledTimes(2);
  });

  it('换了账户就不再听原来的客户端，也忘了记下的事件', () => {
    const { client, registry } = harness();
    const message = undecryptable(client, 'UNSIGNED_SENDER_DEVICE');
    client.emit(MATRIX_EVENT_DECRYPTED, message.asEvent());

    const next = new FakeClient();
    registry.replace(next.asClient());
    expect(client.listeners(MATRIX_EVENT_DECRYPTED)).toBe(0);
    expect(client.listeners(CRYPTO_EVENT_DEVICES_UPDATED)).toBe(0);
    expect(next.listeners(CRYPTO_EVENT_DEVICES_UPDATED)).toBe(1);

    next.emit(CRYPTO_EVENT_DEVICES_UPDATED, [AGENT]);
    expect(message.event.attemptDecryption).not.toHaveBeenCalled();
  });
});
