import type { MatrixClient, MatrixEvent } from 'matrix-js-sdk';
import type { CryptoBackend } from 'matrix-js-sdk/lib/common-crypto/CryptoBackend.js';

import type { MatrixClientSource } from '@/shared/matrix/matrix-client-registry';
import {
  CRYPTO_EVENT_DEVICES_UPDATED,
  CRYPTO_EVENT_USER_TRUST_STATUS_CHANGED,
  MATRIX_EVENT_DECRYPTED,
} from '@/shared/matrix/matrix-sdk-enums';

/**
 * 只因为“发送的设备没签名、不认识”解不开的：这台设备手上的设备列表可能还是旧的。
 * 别的原因（缺密钥、发在加入之前）有房间密钥找回去管，签名身份换过的那种不是过时，是真变了。
 */
const retryCodes: ReadonlySet<string> = new Set([
  'UNSIGNED_SENDER_DEVICE',
  'UNKNOWN_SENDER_DEVICE',
]);
/** 设备列表一直没更新时，过这么久自己再解一次，只一次。 */
const FALLBACK_RETRY_MS = 15_000;
/** 每个发送方最多记这么多条，免得一个真没签名的设备把内存撑大。 */
const MAX_EVENTS_PER_SENDER = 200;

export type MatrixSenderTrustRetryOptions = {
  readonly fallbackRetryMs?: number;
};

/**
 * 新 Agent 第一次说话前才建好签名身份（matrix-adapter 的 `ensure_identity`）。人的设备在它进房间时就记下了它
 * 还没签名的设备，服务器通知“它的设备变了”、这边重新查完以前，那一条就被判成“发送的设备没签名”。
 * SDK 只在收到房间密钥时重试解密，设备列表更新以后不重试，那一条就一直显示解不开（2026-11-16 子房间的
 * 无头验收抓到）。所以记下这样解不开的事件，发送方的设备或签名身份一更新就再解一次；
 * 都没等到的，过一会儿自己再解一次。Agent 那边对应的是收到未签名设备的消息时刷新后再判一次（#116）。
 */
export class MatrixSenderTrustRetry {
  readonly #fallbackRetryMs: number;
  readonly #waiting = new Map<string, Set<MatrixEvent>>();
  /** 已经由定时器再解过一次的，不再定时：真没签名的设备不该每隔一会儿解一遍。 */
  #fallbackDone = new WeakSet<MatrixEvent>();
  #client: MatrixClient | null = null;
  #detach: (() => void) | null = null;
  #timer: ReturnType<typeof setTimeout> | null = null;

  constructor(clients: MatrixClientSource, options: MatrixSenderTrustRetryOptions = {}) {
    this.#fallbackRetryMs = options.fallbackRetryMs ?? FALLBACK_RETRY_MS;
    clients.subscribe(() => {
      this.#bind(clients.current());
    });
    this.#bind(clients.current());
  }

  /** 换了账户（或退出）就丢掉记下的事件：它们属于原来那台设备的时间线。 */
  #bind(client: MatrixClient | null): void {
    if (client === this.#client) return;
    this.#detach?.();
    this.#detach = null;
    this.#client = client;
    this.#waiting.clear();
    this.#fallbackDone = new WeakSet();
    if (this.#timer !== null) clearTimeout(this.#timer);
    this.#timer = null;
    if (client === null) return;
    const onDecrypted = (event: MatrixEvent): void => {
      this.#note(client, event);
    };
    const onDevices = (userIds: string[]): void => {
      this.#retry(client, userIds);
    };
    const onTrust = (userId: string): void => {
      this.#retry(client, [userId]);
    };
    client.on(MATRIX_EVENT_DECRYPTED, onDecrypted);
    client.on(CRYPTO_EVENT_DEVICES_UPDATED, onDevices);
    client.on(CRYPTO_EVENT_USER_TRUST_STATUS_CHANGED, onTrust);
    this.#detach = () => {
      client.off(MATRIX_EVENT_DECRYPTED, onDecrypted);
      client.off(CRYPTO_EVENT_DEVICES_UPDATED, onDevices);
      client.off(CRYPTO_EVENT_USER_TRUST_STATUS_CHANGED, onTrust);
    };
  }

  #note(client: MatrixClient, event: MatrixEvent): void {
    if (!event.isDecryptionFailure()) return;
    const code = event.decryptionFailureReason;
    const sender = event.getSender();
    if (code === null || !retryCodes.has(code) || sender === undefined) return;
    const events = this.#waiting.get(sender) ?? new Set<MatrixEvent>();
    if (events.size >= MAX_EVENTS_PER_SENDER) return;
    events.add(event);
    this.#waiting.set(sender, events);
    if (this.#fallbackDone.has(event) || this.#timer !== null) return;
    this.#timer = setTimeout(() => {
      this.#timer = null;
      if (this.#client !== client) return;
      const senders = [...this.#waiting.keys()];
      for (const pending of this.#waiting.values()) {
        for (const waiting of pending) this.#fallbackDone.add(waiting);
      }
      this.#retry(client, senders);
    }, this.#fallbackRetryMs);
  }

  #retry(client: MatrixClient, userIds: readonly string[]): void {
    if (this.#client !== client) return;
    // getCrypto() 交出的就是 SDK 自己解密用的那一个；只有它的重试入口能把已经判失败的事件再解一次。
    const crypto = client.getCrypto() as CryptoBackend | undefined;
    if (crypto === undefined) return;
    for (const userId of userIds) {
      const events = this.#waiting.get(userId);
      if (events === undefined) continue;
      this.#waiting.delete(userId);
      for (const event of events) {
        // 再失败的话 SDK 照样发出“解密完了”，又记回来，等下一次更新。
        if (event.isDecryptionFailure()) {
          void event.attemptDecryption(crypto, { isRetry: true }).catch(ignoreFailure);
        }
      }
    }
  }
}

function ignoreFailure(): void {
  // 解不开记在事件自己身上，界面照旧汇总成提示。
}
