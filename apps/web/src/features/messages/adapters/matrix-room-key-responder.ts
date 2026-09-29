import type {
  IMegolmSessionData,
  MatrixClient,
  ReceivedToDeviceMessage,
  Room,
} from 'matrix-js-sdk';
import type { CryptoApi } from 'matrix-js-sdk/lib/crypto-api/index.js';
import { z } from 'zod';

import { BrowserUuidV7Factory, type UuidV7Factory } from '@/shared/ids/browser-uuid-v7-factory';
import type { MatrixClientSource } from '@/shared/matrix/matrix-client-registry';
import {
  CLIENT_EVENT_RECEIVED_TO_DEVICE_MESSAGE,
  HISTORY_VISIBILITY_SHARED,
  HISTORY_VISIBILITY_WORLD_READABLE,
} from '@/shared/matrix/matrix-sdk-enums';

import {
  isAgentRoomAgent,
  roomKeyRequestEventType,
  roomKeysEventType,
} from './matrix-room-key-recovery';

/** 同一台请求设备在同一个房间里，隔多久才再答一次。 */
const REPEAT_INTERVAL_MS = 10 * 60 * 1_000;
/** 这台设备每分钟最多答几次。 */
const ANSWERS_PER_MINUTE = 20;
const MINUTE_MS = 60 * 1_000;
/** 每条应答最多带几个密钥，与协议的上限一致。 */
const KEYS_PER_MESSAGE = 20;
/**
 * SDK 只能一次导出这台设备的全部房间密钥。几个 Agent 同时进房间会接连发来请求，
 * 这么短的时间里用同一份导出回答，不必各导出一遍。
 */
const EXPORT_REUSE_MS = 5_000;

const keyLikeSchema = z.string().regex(/^[A-Za-z0-9+/]{43}$/u);
const requestSchema = z.object({
  schemaVersion: z.literal('1.0'),
  eventType: z.literal(roomKeyRequestEventType),
  id: z.string().regex(/^[0-9a-f]{8}-[0-9a-f]{4}-7[0-9a-f]{3}-[89ab][0-9a-f]{3}-[0-9a-f]{12}$/u),
  createdAt: z.string(),
  roomId: z
    .string()
    .min(4)
    .max(255)
    .regex(/^![^:]+:[^:]+$/u),
  sessionIds: z.array(keyLikeSchema).min(1).max(100),
});

type RoomKeyRequest = z.infer<typeof requestSchema>;

export type MatrixRoomKeyResponderOptions = {
  readonly ids?: UuidV7Factory;
  readonly now?: () => number;
};

/**
 * 新请进房间的 Agent 解不开加入前的消息时，会请发消息的那台设备重发它建的房间密钥
 * （设计见 `specs/room-key-recovery/pre-join-history.md`）。这里是人的网页端和桌面端那一头：
 * 只重发这台设备自己建的会话，只发回经 Olm 发来请求的那台 Agent 设备。
 *
 * 请求必须全部满足才回答，否则一声不吭：请求者此刻在房间里、是 Agent Room 的 Agent，请求设备
 * 由主人签名、就是经 Olm 发来请求的那一台，房间历史对成员开放（这时服务器本来就给成员看加入前
 * 的密文）。同一台设备在同一个房间十分钟答一次，这台设备每分钟最多答 20 次。
 */
export class MatrixRoomKeyResponder {
  readonly #ids: UuidV7Factory;
  readonly #now: () => number;
  readonly #lastAnswer = new Map<string, number>();
  #recent: number[] = [];
  #export: { readonly at: number; readonly keys: Promise<IMegolmSessionData[]> } | null = null;
  #client: MatrixClient | null = null;
  #detach: (() => void) | null = null;

  constructor(clients: MatrixClientSource, options: MatrixRoomKeyResponderOptions = {}) {
    this.#ids = options.ids ?? new BrowserUuidV7Factory();
    this.#now = options.now ?? Date.now;
    clients.subscribe(() => {
      this.#bind(clients.current());
    });
    this.#bind(clients.current());
  }

  /** 换了账户（或退出）就不再听旧客户端，频率和缓存的导出也一并丢掉。 */
  #bind(client: MatrixClient | null): void {
    if (client === this.#client) return;
    this.#detach?.();
    this.#detach = null;
    this.#client = client;
    this.#lastAnswer.clear();
    this.#recent = [];
    this.#export = null;
    if (client === null) return;
    const onToDevice = (received: ReceivedToDeviceMessage): void => {
      void this.#answer(client, received).catch(ignoreFailure);
    };
    client.on(CLIENT_EVENT_RECEIVED_TO_DEVICE_MESSAGE, onToDevice);
    this.#detach = () => {
      client.off(CLIENT_EVENT_RECEIVED_TO_DEVICE_MESSAGE, onToDevice);
    };
  }

  async #answer(client: MatrixClient, received: ReceivedToDeviceMessage): Promise<void> {
    const { encryptionInfo, message } = received;
    if (message.type !== roomKeyRequestEventType || encryptionInfo === null) return;
    const deviceId = encryptionInfo.senderDevice;
    if (deviceId === undefined || encryptionInfo.sender !== message.sender) return;
    const parsed = requestSchema.safeParse(message.content);
    if (!parsed.success) return;
    const request = parsed.data;
    const room = client.getRoom(request.roomId);
    if (room?.getMyMembership() !== 'join' || !sharesHistory(room)) return;
    // 后面要查设备、导出房间密钥，先扣频率，免得被一串请求放大。
    if (!this.#admit(message.sender, deviceId, request.roomId)) return;
    if (
      room.getMember(message.sender)?.membership !== 'join' ||
      !isAgentRoomAgent(client, request.roomId, message.sender)
    ) {
      return;
    }
    const crypto = client.getCrypto();
    if (crypto === undefined) return;
    // 请求设备必须由主人签名，而且就是经 Olm 发来请求的那一台。
    const device = (await crypto.getUserDeviceInfo([message.sender], true))
      .get(message.sender)
      ?.get(deviceId);
    if (
      device?.getIdentityKey() !== encryptionInfo.senderCurve25519KeyBase64 ||
      (await crypto.getDeviceVerificationStatus(message.sender, deviceId))?.signedByOwner !== true
    ) {
      return;
    }
    const own = await crypto.getOwnDeviceKeys();
    const keys = ownSessions(await this.#exportedKeys(crypto), request, own.curve25519);
    if (keys.length === 0 || this.#client !== client) return;
    for (let start = 0; start < keys.length; start += KEYS_PER_MESSAGE) {
      const payload = {
        createdAt: new Date(this.#now()).toISOString(),
        eventType: roomKeysEventType,
        id: this.#ids.next(),
        keys: keys
          .slice(start, start + KEYS_PER_MESSAGE)
          .map((key) => ({ sessionId: key.session_id, sessionKey: key.session_key })),
        requestId: request.id,
        roomId: request.roomId,
        schemaVersion: '1.0',
        senderEd25519Key: own.ed25519,
        senderKey: own.curve25519,
      };
      // 用和这台设备之间最新的 Olm 会话加密，也就是 Agent 刚才发请求时建（或用）的那条。
      const toDevice = await crypto.encryptToDeviceMessages(
        roomKeysEventType,
        [{ deviceId, userId: message.sender }],
        payload,
      );
      await client.queueToDevice(toDevice);
    }
  }

  #admit(userId: string, deviceId: string, roomId: string): boolean {
    const now = this.#now();
    this.#recent = this.#recent.filter((at) => now - at < MINUTE_MS);
    if (this.#recent.length >= ANSWERS_PER_MINUTE) return false;
    for (const [key, at] of this.#lastAnswer) {
      if (now - at >= REPEAT_INTERVAL_MS) this.#lastAnswer.delete(key);
    }
    const key = `${userId}\u0000${deviceId}\u0000${roomId}`;
    if (this.#lastAnswer.has(key)) return false;
    this.#lastAnswer.set(key, now);
    this.#recent.push(now);
    return true;
  }

  #exportedKeys(crypto: CryptoApi): Promise<IMegolmSessionData[]> {
    const now = this.#now();
    if (this.#export === null || now - this.#export.at >= EXPORT_REUSE_MS) {
      this.#export = { at: now, keys: crypto.exportRoomKeys() };
    }
    return this.#export.keys;
  }
}

function sharesHistory(room: Room): boolean {
  const visibility = room.getHistoryVisibility();
  return (
    visibility === HISTORY_VISIBILITY_SHARED || visibility === HISTORY_VISIBILITY_WORLD_READABLE
  );
}

/** 只要这个房间里、由这台设备建的、请求点名的会话：绝不把别人的会话转发出去。 */
function ownSessions(
  keys: readonly IMegolmSessionData[],
  request: RoomKeyRequest,
  ownCurve25519: string,
): IMegolmSessionData[] {
  const wanted = new Set(request.sessionIds);
  return keys.filter(
    (key) =>
      key.room_id === request.roomId &&
      key.sender_key === ownCurve25519 &&
      wanted.has(key.session_id),
  );
}

function ignoreFailure(error: unknown): void {
  void error;
}
