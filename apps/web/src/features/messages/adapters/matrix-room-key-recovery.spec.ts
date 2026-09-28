import type { MatrixClient } from 'matrix-js-sdk';
import { describe, expect, it, vi } from 'vitest';

import { matrixAgentStatusEventType } from '@/features/lobby/adapters/matrix-lobby-source';
import { MatrixClientRegistry } from '@/shared/matrix/matrix-client-registry';

import {
  MatrixRoomKeyRecovery,
  roomKeyRequestEventType,
  roomKeysEventType,
} from './matrix-room-key-recovery';

const ROOM = '!private:agent-room.test';
const ME = '@human:agent-room.test';
const MY_DEVICE = 'HUMANDEVICE';
const AGENT = '@agent:agent-room.test';
const AGENT_DEVICE = 'AGENTDEVICE';
const STRANGER = '@stranger:agent-room.test';
const AGENT_CURVE = 'c'.repeat(43);
const AGENT_ED25519 = 'e'.repeat(43);
const SESSION_A = `A${'a'.repeat(42)}`;
const SESSION_B = `B${'b'.repeat(42)}`;
const REQUEST_ID = '01990d9e-8400-7000-8000-000000000101';

type Handler = (...args: unknown[]) => void;

class FakeCrypto {
  ownSigned = true;
  agentSigned = true;
  agentKeys = { curve: AGENT_CURVE, ed25519: AGENT_ED25519 };
  readonly encryptToDeviceMessages = vi.fn(
    (eventType: string, devices: { userId: string; deviceId: string }[], payload: object) =>
      Promise.resolve({ batch: devices.map((device) => ({ ...device, payload })), eventType }),
  );
  readonly importRoomKeys = vi.fn((keys: unknown[]) => {
    void keys;
    return Promise.resolve();
  });

  getDeviceVerificationStatus(userId: string, deviceId: string) {
    const signed =
      userId === ME && deviceId === MY_DEVICE
        ? this.ownSigned
        : userId === AGENT && deviceId === AGENT_DEVICE && this.agentSigned;
    return Promise.resolve({ signedByOwner: signed });
  }

  getUserDeviceInfo(userIds: string[]) {
    const devices = new Map<string, Map<string, unknown>>();
    for (const userId of userIds) {
      if (userId !== AGENT) continue;
      devices.set(
        AGENT,
        new Map([
          [
            AGENT_DEVICE,
            {
              getFingerprint: () => this.agentKeys.ed25519,
              getIdentityKey: () => this.agentKeys.curve,
            },
          ],
        ]),
      );
    }
    return Promise.resolve(devices);
  }
}

class FakeClient {
  readonly crypto = new FakeCrypto();
  readonly queueToDevice = vi.fn(() => Promise.resolve());
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

  getCrypto(): FakeCrypto {
    return this.crypto;
  }

  getUserId(): string {
    return ME;
  }

  getDeviceId(): string {
    return MY_DEVICE;
  }

  getRoom(roomId: string) {
    if (roomId !== ROOM) return null;
    const state = {
      getStateEvents: (type: string) =>
        type === matrixAgentStatusEventType ? [{ getSender: () => AGENT }] : [],
    };
    return { getLiveTimeline: () => ({ getState: () => state }) };
  }

  asClient(): MatrixClient {
    return this as unknown as MatrixClient;
  }
}

function undecryptable(
  sessionId: string,
  options: { sender?: string; reason?: string | null; deviceId?: string | null } = {},
) {
  return {
    decryptionFailureReason:
      options.reason === undefined ? 'MEGOLM_UNKNOWN_INBOUND_SESSION_ID' : options.reason,
    getRoomId: () => ROOM,
    getSender: () => options.sender ?? AGENT,
    getWireContent: () => ({
      algorithm: 'm.megolm.v1.aes-sha2',
      ciphertext: 'opaque',
      session_id: sessionId,
      ...(options.deviceId === null ? {} : { device_id: options.deviceId ?? AGENT_DEVICE }),
    }),
    isDecryptionFailure: () => true,
  };
}

function harness() {
  const client = new FakeClient();
  const registry = new MatrixClientRegistry();
  registry.replace(client.asClient());
  let id = 0;
  const recovery = new MatrixRoomKeyRecovery(registry, {
    flushDelayMs: 0,
    ids: {
      next: () =>
        (id += 1) === 1 ? REQUEST_ID : `01990d9e-8400-7000-8000-00000000010${String(id)}`,
    },
    now: () => Date.UTC(2026, 8, 28, 14),
  });
  return { client, recovery, registry };
}

function response(overrides: Record<string, unknown> = {}) {
  return {
    createdAt: '2026-09-28T14:00:01.000Z',
    eventType: roomKeysEventType,
    id: '01990d9e-8400-7000-8000-000000000201',
    keys: [
      { sessionId: SESSION_A, sessionKey: 'k'.repeat(220) },
      { sessionId: SESSION_B, sessionKey: 'm'.repeat(220) },
    ],
    requestId: REQUEST_ID,
    roomId: ROOM,
    schemaVersion: '1.0',
    senderEd25519Key: AGENT_ED25519,
    senderKey: AGENT_CURVE,
    ...overrides,
  };
}

function delivered(content: object, sender = AGENT, encrypted = true) {
  return {
    encryptionInfo: encrypted
      ? {
          sender,
          senderCurve25519KeyBase64: AGENT_CURVE,
          senderDevice: AGENT_DEVICE,
          senderVerified: false,
        }
      : null,
    message: { content, sender, type: roomKeysEventType },
  };
}

async function requestedTwoSessions() {
  const setup = harness();
  setup.client.emit('Event.decrypted', undecryptable(SESSION_A));
  setup.client.emit('Event.decrypted', undecryptable(SESSION_B));
  await vi.waitFor(() => {
    expect(setup.client.queueToDevice).toHaveBeenCalledOnce();
  });
  return setup;
}

describe('请 Agent 重发房间密钥', () => {
  it('同一台 Agent 设备同一个房间缺的会话攒成一条经 Olm 加密的请求', async () => {
    const { client, recovery } = await requestedTwoSessions();

    expect(client.crypto.encryptToDeviceMessages).toHaveBeenCalledExactlyOnceWith(
      roomKeyRequestEventType,
      [{ deviceId: AGENT_DEVICE, userId: AGENT }],
      {
        createdAt: '2026-09-28T14:00:00.000Z',
        eventType: roomKeyRequestEventType,
        id: REQUEST_ID,
        roomId: ROOM,
        schemaVersion: '1.0',
        sessionIds: [SESSION_A, SESSION_B],
      },
    );
    expect(recovery.pending(ROOM)).toBe(2);
    expect(recovery.pending('!other:agent-room.test')).toBe(0);
  });

  it('不是缺密钥、不是 Agent、自己发的、一小时内请求过的都不再请求', async () => {
    const { client } = await requestedTwoSessions();
    client.emit('Event.decrypted', undecryptable(SESSION_A));
    client.emit(
      'Event.decrypted',
      undecryptable(`C${'c'.repeat(42)}`, { reason: 'MEGOLM_KEY_WITHHELD' }),
    );
    client.emit('Event.decrypted', undecryptable(`D${'d'.repeat(42)}`, { sender: STRANGER }));
    client.emit('Event.decrypted', undecryptable(`E${'e'.repeat(42)}`, { sender: ME }));
    await new Promise((settle) => setTimeout(settle, 20));

    expect(client.crypto.encryptToDeviceMessages).toHaveBeenCalledOnce();
  });

  it('这台设备还没由主人签名时不请求（Agent 反正不会回答）', async () => {
    const { client } = harness();
    client.crypto.ownSigned = false;
    client.emit('Event.decrypted', undecryptable(SESSION_A));
    await new Promise((settle) => setTimeout(settle, 20));

    expect(client.crypto.encryptToDeviceMessages).not.toHaveBeenCalled();
  });

  it('核对来源后按导出格式导入，等待中的会话随之清零', async () => {
    const { client, recovery } = await requestedTwoSessions();
    const changed = vi.fn();
    recovery.subscribe(changed);

    client.emit('receivedToDeviceMessage', delivered(response()));

    await vi.waitFor(() => {
      expect(client.crypto.importRoomKeys).toHaveBeenCalledOnce();
    });
    expect(client.crypto.importRoomKeys.mock.calls[0]?.[0]).toEqual([
      {
        algorithm: 'm.megolm.v1.aes-sha2',
        forwarding_curve25519_key_chain: [],
        room_id: ROOM,
        sender_claimed_keys: { ed25519: AGENT_ED25519 },
        sender_key: AGENT_CURVE,
        session_id: SESSION_A,
        session_key: 'k'.repeat(220),
      },
      {
        algorithm: 'm.megolm.v1.aes-sha2',
        forwarding_curve25519_key_chain: [],
        room_id: ROOM,
        sender_claimed_keys: { ed25519: AGENT_ED25519 },
        sender_key: AGENT_CURVE,
        session_id: SESSION_B,
        session_key: 'm'.repeat(220),
      },
    ]);
    expect(recovery.pending(ROOM)).toBe(0);
    expect(changed).toHaveBeenCalled();
  });

  it.each([
    ['明文送来', () => delivered(response(), AGENT, false)],
    [
      '不是我们发的请求',
      () => delivered(response({ requestId: '01990d9e-8400-7000-8000-000000000999' })),
    ],
    ['别人冒充应答', () => delivered(response(), STRANGER)],
    ['别的房间', () => delivered(response({ roomId: '!other:agent-room.test' }))],
    [
      '夹带没请求的会话',
      () =>
        delivered(
          response({ keys: [{ sessionId: `Z${'z'.repeat(42)}`, sessionKey: 'k'.repeat(220) }] }),
        ),
    ],
    ['外层公钥不是这台设备', () => delivered(response({ senderKey: 'x'.repeat(43) }))],
  ])('%s的应答整条丢掉', async (_label, make) => {
    const { client } = await requestedTwoSessions();

    client.emit('receivedToDeviceMessage', make());
    await new Promise((settle) => setTimeout(settle, 20));

    expect(client.crypto.importRoomKeys).not.toHaveBeenCalled();
  });

  it('发送设备没由主人签名、或设备密钥对不上时不导入', async () => {
    const unsigned = await requestedTwoSessions();
    unsigned.client.crypto.agentSigned = false;
    unsigned.client.emit('receivedToDeviceMessage', delivered(response()));

    const mismatched = await requestedTwoSessions();
    mismatched.client.crypto.agentKeys = { curve: AGENT_CURVE, ed25519: 'f'.repeat(43) };
    mismatched.client.emit('receivedToDeviceMessage', delivered(response()));
    await new Promise((settle) => setTimeout(settle, 20));

    expect(unsigned.client.crypto.importRoomKeys).not.toHaveBeenCalled();
    expect(mismatched.client.crypto.importRoomKeys).not.toHaveBeenCalled();
  });

  it('换了账户就丢掉等待中的请求，也不再听旧客户端', async () => {
    const { client, recovery, registry } = await requestedTwoSessions();

    registry.replace(null);

    expect(recovery.pending(ROOM)).toBe(0);
    expect(client.listeners('Event.decrypted')).toBe(0);
    expect(client.listeners('receivedToDeviceMessage')).toBe(0);
  });
});
