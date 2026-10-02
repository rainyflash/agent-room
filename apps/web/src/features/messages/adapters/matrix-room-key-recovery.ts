import type { MatrixClient, MatrixEvent, ReceivedToDeviceMessage } from 'matrix-js-sdk';
import { z } from 'zod';

import { matrixAgentStatusEventType } from '@/features/lobby/adapters/matrix-lobby-source';
import { BrowserUuidV7Factory, type UuidV7Factory } from '@/shared/ids/browser-uuid-v7-factory';
import type { MatrixClientSource } from '@/shared/matrix/matrix-client-registry';
import {
  CLIENT_EVENT_RECEIVED_TO_DEVICE_MESSAGE,
  CRYPTO_EVENT_DEVICES_UPDATED,
  CRYPTO_EVENT_USER_TRUST_STATUS_CHANGED,
  DIRECTION_FORWARD,
  MATRIX_EVENT_DECRYPTED,
} from '@/shared/matrix/matrix-sdk-enums';

export const roomKeyRequestEventType = 'io.github.rainyflash.agentroom.room_keys.request.v1';
export const roomKeysEventType = 'io.github.rainyflash.agentroom.room_keys.v1';

/**
 * 值得请 Agent 重发的解密失败原因码：#202 归为“没收到密钥”和“发在加入之前”的那几种，
 * 加上发送方拒绝分发的两种。拒绝多半因为这台设备当时还没签好；签好以后再请求，Agent 按同样的规则重新判断。
 */
const recoverableCodes: ReadonlySet<string> = new Set([
  'MEGOLM_UNKNOWN_INBOUND_SESSION_ID',
  'OLM_UNKNOWN_MESSAGE_INDEX',
  'HISTORICAL_MESSAGE_NO_KEY_BACKUP',
  'HISTORICAL_MESSAGE_BACKUP_UNCONFIGURED',
  'HISTORICAL_MESSAGE_WORKING_BACKUP',
  'HISTORICAL_MESSAGE_USER_NOT_JOINED',
  'MEGOLM_KEY_WITHHELD',
  'MEGOLM_KEY_WITHHELD_FOR_UNVERIFIED_DEVICE',
]);
/** 攒一会儿再发：进房间时一批事件同时解密失败，合成一条请求。 */
const FLUSH_DELAY_MS = 2_000;
/** 同一个会话一小时内只请求一次；请求也只等这么久。 */
const REQUEST_TTL_MS = 60 * 60 * 1_000;
const MAX_SESSIONS_PER_REQUEST = 100;

const keyLikeSchema = z.string().regex(/^[A-Za-z0-9+/]{43}$/u);
const roomKeysSchema = z.object({
  schemaVersion: z.literal('1.0'),
  eventType: z.literal(roomKeysEventType),
  requestId: z.string(),
  roomId: z.string(),
  senderKey: keyLikeSchema,
  senderEd25519Key: keyLikeSchema,
  keys: z
    .array(
      z.strictObject({
        sessionId: keyLikeSchema,
        sessionKey: z
          .string()
          .min(16)
          .max(1_024)
          .regex(/^[A-Za-z0-9+/]+={0,2}$/u),
      }),
    )
    .min(1)
    .max(20),
});

type QueuedRequest = {
  readonly roomId: string;
  readonly userId: string;
  readonly deviceId: string | null;
  readonly sessionIds: Set<string>;
};

type PendingRequest = {
  readonly roomId: string;
  readonly userId: string;
  readonly sessionIds: Set<string>;
  readonly expiresAt: number;
};

/** 界面用：某个房间里还在等 Agent 重发的会话有几个，因为这台设备还没签好而扣着没发的又有几个。 */
export type RoomKeyRecoveryStatus = {
  pending(roomId: string): number;
  awaitingSigning(roomId: string): number;
  subscribe(listener: () => void): () => void;
};

export type MatrixRoomKeyRecoveryOptions = {
  readonly flushDelayMs?: number;
  readonly ids?: UuidV7Factory;
  readonly now?: () => number;
};

/**
 * 人的设备缺密钥解不开 Agent 的消息时，请那个 Agent 重发它自己建的房间密钥
 * （设计见 `specs/room-key-recovery/design.md`）。
 *
 * 请求经 Olm 加密发出：和那台 Agent 设备之间没有能用的会话时，SDK 先领它的一次性密钥建一条新的，
 * 坏掉的通道就此换掉。应答只认我们请求过的、由对方主人签名的那台设备发来的、它自己建的会话，
 * 核对后导入；SDK 会对等着这些密钥的事件自动重试解密。
 */
export class MatrixRoomKeyRecovery implements RoomKeyRecoveryStatus {
  readonly #flushDelayMs: number;
  readonly #ids: UuidV7Factory;
  readonly #now: () => number;
  readonly #listeners = new Set<() => void>();
  readonly #queued = new Map<string, QueuedRequest>();
  /** 这台设备还没由主人签名时攒下的请求：Agent 现在不会回答，签名以后一起发。 */
  readonly #parked = new Map<string, QueuedRequest>();
  readonly #requestedAt = new Map<string, number>();
  readonly #pending = new Map<string, PendingRequest>();
  #client: MatrixClient | null = null;
  #detach: (() => void) | null = null;
  #timer: ReturnType<typeof setTimeout> | null = null;

  constructor(clients: MatrixClientSource, options: MatrixRoomKeyRecoveryOptions = {}) {
    this.#flushDelayMs = options.flushDelayMs ?? FLUSH_DELAY_MS;
    this.#ids = options.ids ?? new BrowserUuidV7Factory();
    this.#now = options.now ?? Date.now;
    clients.subscribe(() => {
      this.#bind(clients.current());
    });
    this.#bind(clients.current());
  }

  pending(roomId: string): number {
    const now = this.#now();
    let count = 0;
    for (const request of this.#pending.values()) {
      if (request.roomId === roomId && request.expiresAt > now) count += request.sessionIds.size;
    }
    return count;
  }

  awaitingSigning(roomId: string): number {
    let count = 0;
    for (const request of this.#parked.values()) {
      if (request.roomId === roomId) count += request.sessionIds.size;
    }
    return count;
  }

  subscribe(listener: () => void): () => void {
    this.#listeners.add(listener);
    return () => {
      this.#listeners.delete(listener);
    };
  }

  /** 换了账户（或退出）就丢掉手上的一切：请求和应答都只对当时那台设备有意义。 */
  #bind(client: MatrixClient | null): void {
    if (client === this.#client) return;
    this.#detach?.();
    this.#detach = null;
    this.#client = client;
    this.#queued.clear();
    this.#parked.clear();
    this.#requestedAt.clear();
    this.#pending.clear();
    if (this.#timer !== null) clearTimeout(this.#timer);
    this.#timer = null;
    this.#notify();
    if (client === null) return;
    const onDecrypted = (event: MatrixEvent): void => {
      this.#noteUndecryptable(client, event);
    };
    const onToDevice = (received: ReceivedToDeviceMessage): void => {
      void this.#receive(client, received).catch(ignoreFailure);
    };
    // 在“安全”页恢复这台设备以后，服务器上的签名要等下一次查设备列表才到本地；两个事件都可能先来。
    const onOwnTrust = (userId: string): void => {
      if (userId === client.getUserId()) void this.#releaseParked(client).catch(ignoreFailure);
    };
    const onDevices = (userIds: string[]): void => {
      const own = client.getUserId();
      if (own !== null && userIds.includes(own)) {
        void this.#releaseParked(client).catch(ignoreFailure);
      }
    };
    client.on(MATRIX_EVENT_DECRYPTED, onDecrypted);
    client.on(CLIENT_EVENT_RECEIVED_TO_DEVICE_MESSAGE, onToDevice);
    client.on(CRYPTO_EVENT_USER_TRUST_STATUS_CHANGED, onOwnTrust);
    client.on(CRYPTO_EVENT_DEVICES_UPDATED, onDevices);
    this.#detach = () => {
      client.off(MATRIX_EVENT_DECRYPTED, onDecrypted);
      client.off(CLIENT_EVENT_RECEIVED_TO_DEVICE_MESSAGE, onToDevice);
      client.off(CRYPTO_EVENT_USER_TRUST_STATUS_CHANGED, onOwnTrust);
      client.off(CRYPTO_EVENT_DEVICES_UPDATED, onDevices);
    };
  }

  #noteUndecryptable(client: MatrixClient, event: MatrixEvent): void {
    if (!event.isDecryptionFailure()) return;
    const code = event.decryptionFailureReason;
    if (code === null || !recoverableCodes.has(code)) return;
    const roomId = event.getRoomId();
    const sender = event.getSender();
    const wire = event.getWireContent() as { session_id?: unknown; device_id?: unknown };
    if (
      roomId === undefined ||
      sender === undefined ||
      sender === client.getUserId() ||
      typeof wire.session_id !== 'string' ||
      !isAgentRoomAgent(client, roomId, sender)
    ) {
      return;
    }
    const sessionKey = `${roomId}\u0000${wire.session_id}`;
    const last = this.#requestedAt.get(sessionKey);
    if (last !== undefined && this.#now() - last < REQUEST_TTL_MS) return;
    this.#requestedAt.set(sessionKey, this.#now());
    const deviceId = typeof wire.device_id === 'string' ? wire.device_id : null;
    const queueKey = `${roomId}\u0000${sender}\u0000${deviceId ?? ''}`;
    const queued = this.#queued.get(queueKey) ?? {
      deviceId,
      roomId,
      sessionIds: new Set<string>(),
      userId: sender,
    };
    queued.sessionIds.add(wire.session_id);
    this.#queued.set(queueKey, queued);
    this.#timer ??= setTimeout(() => {
      this.#timer = null;
      void this.#flush(client);
    }, this.#flushDelayMs);
  }

  async #flush(client: MatrixClient): Promise<void> {
    const batches = [...this.#queued];
    this.#queued.clear();
    const signed = await ownDeviceSigned(client);
    if (signed === null || this.#client !== client) return;
    if (signed) {
      await this.#send(client, batches);
      return;
    }
    // Agent 只回答由主人签名的设备，现在问也白问；先扣着，登录后的自动签名签好这台设备就发出去。
    for (const [key, batch] of batches) {
      const parked = this.#parked.get(key);
      if (parked === undefined) this.#parked.set(key, batch);
      else for (const sessionId of batch.sessionIds) parked.sessionIds.add(sessionId);
    }
    this.#notify();
  }

  /** 这台设备的签名可能刚变：签好了，就把扣着的请求发出去。 */
  async #releaseParked(client: MatrixClient): Promise<void> {
    if (this.#parked.size === 0 || (await ownDeviceSigned(client)) !== true) return;
    if (this.#client !== client || this.#parked.size === 0) return;
    const batches = [...this.#parked];
    this.#parked.clear();
    this.#notify();
    await this.#send(client, batches);
  }

  async #send(
    client: MatrixClient,
    batches: readonly (readonly [string, QueuedRequest])[],
  ): Promise<void> {
    for (const [, batch] of batches) {
      if (this.#client !== client) return;
      try {
        await this.#request(client, batch);
      } catch {
        // 发不出去就算了：一小时后这些消息再解不开时会再请求。
      }
    }
  }

  async #request(client: MatrixClient, batch: QueuedRequest): Promise<void> {
    const crypto = client.getCrypto();
    if (crypto === undefined) return;
    const devices =
      batch.deviceId === null
        ? [
            ...((await crypto.getUserDeviceInfo([batch.userId], true)).get(batch.userId)?.keys() ??
              []),
          ]
        : [batch.deviceId];
    if (devices.length === 0) return;
    const ids = [...batch.sessionIds];
    for (let start = 0; start < ids.length; start += MAX_SESSIONS_PER_REQUEST) {
      const sessionIds = ids.slice(start, start + MAX_SESSIONS_PER_REQUEST);
      const id = this.#ids.next();
      const payload = {
        createdAt: new Date(this.#now()).toISOString(),
        eventType: roomKeyRequestEventType,
        id,
        roomId: batch.roomId,
        schemaVersion: '1.0',
        sessionIds,
      };
      this.#pending.set(id, {
        expiresAt: this.#now() + REQUEST_TTL_MS,
        roomId: batch.roomId,
        sessionIds: new Set(sessionIds),
        userId: batch.userId,
      });
      this.#notify();
      const toDevice = await crypto.encryptToDeviceMessages(
        roomKeyRequestEventType,
        devices.map((deviceId) => ({ deviceId, userId: batch.userId })),
        payload,
      );
      await client.queueToDevice(toDevice);
    }
  }

  async #receive(client: MatrixClient, received: ReceivedToDeviceMessage): Promise<void> {
    const { encryptionInfo, message } = received;
    if (message.type !== roomKeysEventType || encryptionInfo === null) return;
    const parsed = roomKeysSchema.safeParse(message.content);
    if (!parsed.success) return;
    const response = parsed.data;
    const request = this.#pending.get(response.requestId);
    const deviceId = encryptionInfo.senderDevice;
    if (
      request === undefined ||
      request.expiresAt <= this.#now() ||
      deviceId === undefined ||
      encryptionInfo.sender !== request.userId ||
      message.sender !== request.userId ||
      response.roomId !== request.roomId ||
      encryptionInfo.senderCurve25519KeyBase64 !== response.senderKey ||
      !response.keys.every((key) => request.sessionIds.has(key.sessionId))
    ) {
      return;
    }
    const crypto = client.getCrypto();
    if (crypto === undefined) return;
    const device = (await crypto.getUserDeviceInfo([request.userId]))
      .get(request.userId)
      ?.get(deviceId);
    if (
      device?.getIdentityKey() !== response.senderKey ||
      device.getFingerprint() !== response.senderEd25519Key ||
      (await crypto.getDeviceVerificationStatus(request.userId, deviceId))?.signedByOwner !== true
    ) {
      return;
    }
    await crypto.importRoomKeys(
      response.keys.map((key) => ({
        algorithm: 'm.megolm.v1.aes-sha2',
        forwarding_curve25519_key_chain: [],
        room_id: response.roomId,
        sender_claimed_keys: { ed25519: response.senderEd25519Key },
        sender_key: response.senderKey,
        session_id: key.sessionId,
        session_key: key.sessionKey,
      })),
    );
    for (const key of response.keys) request.sessionIds.delete(key.sessionId);
    if (request.sessionIds.size === 0) this.#pending.delete(response.requestId);
    this.#notify();
  }

  #notify(): void {
    for (const listener of this.#listeners) listener();
  }
}

/** 这台设备是否由主人签名；加密还没准备好时是 null。 */
async function ownDeviceSigned(client: MatrixClient): Promise<boolean | null> {
  const crypto = client.getCrypto();
  const userId = client.getUserId();
  const deviceId = client.getDeviceId();
  if (crypto === undefined || userId === null || deviceId === null) return null;
  return (await crypto.getDeviceVerificationStatus(userId, deviceId))?.signedByOwner === true;
}

/** 房间里有它发的 Agent 在线状态事件，才是 Agent Room 的 Agent；不看用户 ID 的写法。 */
export function isAgentRoomAgent(client: MatrixClient, roomId: string, userId: string): boolean {
  return (
    client
      .getRoom(roomId)
      ?.getLiveTimeline()
      .getState(DIRECTION_FORWARD)
      ?.getStateEvents(matrixAgentStatusEventType)
      .some((event) => event.getSender() === userId) ?? false
  );
}

function ignoreFailure(error: unknown): void {
  void error;
}
