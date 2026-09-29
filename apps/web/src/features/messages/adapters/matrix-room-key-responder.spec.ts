import type { MatrixClient } from 'matrix-js-sdk';
import { describe, expect, it, vi } from 'vitest';

import { matrixAgentStatusEventType } from '@/features/lobby/adapters/matrix-lobby-source';
import { MatrixClientRegistry } from '@/shared/matrix/matrix-client-registry';

import { roomKeyRequestEventType, roomKeysEventType } from './matrix-room-key-recovery';
import { MatrixRoomKeyResponder } from './matrix-room-key-responder';

const ROOM = '!private:agent-room.test';
const OTHER_ROOM = '!other:agent-room.test';
const MY_CURVE = 'h'.repeat(43);
const MY_ED25519 = 'i'.repeat(43);
const AGENT = '@agent:agent-room.test';
const AGENT_DEVICE = 'AGENTDEVICE';
const SECOND_DEVICE = 'SECONDDEVICE';
const STRANGER = '@stranger:agent-room.test';
const OTHER_CURVE = 'o'.repeat(43);
const REQUEST_ID = '01990d9e-8400-7000-8000-000000000301';
const ANSWER_ID = '01990d9e-8400-7000-8000-000000000401';

type Handler = (...args: unknown[]) => void;

function session(index: number): string {
  return `S${String(index).padStart(42, '0')}`;
}

function curveOf(deviceId: string): string {
  return deviceId === SECOND_DEVICE ? 'd'.repeat(43) : 'c'.repeat(43);
}

function exported(sessionId: string, options: { room?: string; senderKey?: string } = {}) {
  return {
    algorithm: 'm.megolm.v1.aes-sha2',
    forwarding_curve25519_key_chain: [],
    room_id: options.room ?? ROOM,
    sender_claimed_keys: { ed25519: MY_ED25519 },
    sender_key: options.senderKey ?? MY_CURVE,
    session_id: sessionId,
    session_key: `key-of-${sessionId}`,
  };
}

class FakeCrypto {
  signed = true;
  /** 请求者名下已知的设备和它们的 Curve25519。 */
  readonly devices = new Map([
    [AGENT_DEVICE, curveOf(AGENT_DEVICE)],
    [SECOND_DEVICE, curveOf(SECOND_DEVICE)],
  ]);
  keys = [exported(session(1)), exported(session(2))];
  readonly exportRoomKeys = vi.fn(() => Promise.resolve(this.keys));
  readonly encryptToDeviceMessages = vi.fn(
    (eventType: string, devices: { userId: string; deviceId: string }[], payload: object) =>
      Promise.resolve({ batch: devices.map((device) => ({ ...device, payload })), eventType }),
  );
  readonly getUserDeviceInfo = vi.fn((userIds: string[]) =>
    Promise.resolve(
      new Map(
        userIds.map((userId) => [
          userId,
          new Map(
            [...this.devices].map(([deviceId, curve]) => [
              deviceId,
              { getIdentityKey: () => curve },
            ]),
          ),
        ]),
      ),
    ),
  );

  getOwnDeviceKeys() {
    return Promise.resolve({ curve25519: MY_CURVE, ed25519: MY_ED25519 });
  }

  getDeviceVerificationStatus(userId: string, deviceId: string) {
    void userId;
    return Promise.resolve({ signedByOwner: this.signed && this.devices.has(deviceId) });
  }
}

class FakeRoom {
  myMembership = 'join';
  historyVisibility = 'shared';
  readonly members = new Map([
    [AGENT, 'join'],
    [STRANGER, 'join'],
  ]);

  getMyMembership(): string {
    return this.myMembership;
  }

  getHistoryVisibility(): string {
    return this.historyVisibility;
  }

  getMember(userId: string) {
    const membership = this.members.get(userId);
    return membership === undefined ? null : { membership };
  }

  getLiveTimeline() {
    // 只有 AGENT 在房间里发过 Agent 在线状态事件。
    const state = {
      getStateEvents: (type: string) =>
        type === matrixAgentStatusEventType ? [{ getSender: () => AGENT }] : [],
    };
    return { getState: () => state };
  }
}

class FakeClient {
  readonly crypto = new FakeCrypto();
  readonly room = new FakeRoom();
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

  getRoom(roomId: string): FakeRoom | null {
    return roomId === ROOM ? this.room : null;
  }

  asClient(): MatrixClient {
    return this as unknown as MatrixClient;
  }
}

function harness() {
  const client = new FakeClient();
  const registry = new MatrixClientRegistry();
  registry.replace(client.asClient());
  const clock = { now: Date.UTC(2026, 8, 29, 9) };
  const responder = new MatrixRoomKeyResponder(registry, {
    ids: { next: () => ANSWER_ID },
    now: () => clock.now,
  });
  return { client, clock, registry, responder };
}

function request(overrides: Record<string, unknown> = {}) {
  return {
    createdAt: '2026-09-29T09:00:00.000Z',
    eventType: roomKeyRequestEventType,
    id: REQUEST_ID,
    roomId: ROOM,
    schemaVersion: '1.0',
    sessionIds: [session(1), session(2)],
    ...overrides,
  };
}

function delivered(
  content: object,
  options: { sender?: string; olmSender?: string; encrypted?: boolean; device?: string } = {},
) {
  const sender = options.sender ?? AGENT;
  const device = options.device ?? AGENT_DEVICE;
  return {
    encryptionInfo:
      options.encrypted === false
        ? null
        : {
            sender: options.olmSender ?? sender,
            senderCurve25519KeyBase64: curveOf(device),
            senderDevice: device,
            senderVerified: false,
          },
    message: { content, sender, type: roomKeyRequestEventType },
  };
}

async function settle(): Promise<void> {
  await new Promise((resolve) => setTimeout(resolve, 20));
}

type SilentCase = {
  readonly label: string;
  readonly message?: () => ReturnType<typeof delivered>;
  readonly prepare?: (client: FakeClient) => void;
};

const silentCases: readonly SilentCase[] = [
  { label: '明文送来', message: () => delivered(request(), { encrypted: false }) },
  {
    label: '外层发送者和 Olm 送达的对不上',
    message: () => delivered(request(), { olmSender: STRANGER }),
  },
  { label: '格式不合', message: () => delivered(request({ sessionIds: [] })) },
  { label: '不认识的房间', message: () => delivered(request({ roomId: OTHER_ROOM })) },
  {
    label: '这台设备已经不在房间里',
    prepare: (client) => {
      client.room.myMembership = 'leave';
    },
  },
  {
    label: '房间历史不对成员开放',
    prepare: (client) => {
      client.room.historyVisibility = 'joined';
    },
  },
  {
    label: '请求者此刻不在房间里',
    prepare: (client) => {
      client.room.members.set(AGENT, 'leave');
    },
  },
  {
    label: '请求者不是 Agent Room 的 Agent',
    message: () => delivered(request(), { sender: STRANGER }),
  },
  {
    label: '请求设备没由主人签名',
    prepare: (client) => {
      client.crypto.signed = false;
    },
  },
  {
    label: '请求设备的 Curve25519 不是经 Olm 发来请求的那台',
    prepare: (client) => {
      client.crypto.devices.set(AGENT_DEVICE, OTHER_CURVE);
    },
  },
  {
    label: '没有点名的、由这台设备建的会话',
    prepare: (client) => {
      client.crypto.keys = [exported(session(1), { senderKey: OTHER_CURVE })];
    },
  },
];

describe('回答新请进房间的 Agent 的房间密钥请求', () => {
  it('只把这台设备在这个房间建的、请求点名的会话经 Olm 发回请求设备', async () => {
    const { client } = harness();
    client.crypto.keys = [
      exported(session(1)),
      exported(session(2)),
      exported(session(3)),
      exported(session(1), { room: OTHER_ROOM }),
      exported(session(2), { senderKey: OTHER_CURVE }),
    ];

    client.emit('receivedToDeviceMessage', delivered(request()));

    await vi.waitFor(() => {
      expect(client.queueToDevice).toHaveBeenCalledOnce();
    });
    expect(client.crypto.encryptToDeviceMessages).toHaveBeenCalledExactlyOnceWith(
      roomKeysEventType,
      [{ deviceId: AGENT_DEVICE, userId: AGENT }],
      {
        createdAt: '2026-09-29T09:00:00.000Z',
        eventType: roomKeysEventType,
        id: ANSWER_ID,
        keys: [
          { sessionId: session(1), sessionKey: `key-of-${session(1)}` },
          { sessionId: session(2), sessionKey: `key-of-${session(2)}` },
        ],
        requestId: REQUEST_ID,
        roomId: ROOM,
        schemaVersion: '1.0',
        senderEd25519Key: MY_ED25519,
        senderKey: MY_CURVE,
      },
    );
  });

  it('多于 20 个会话时分几条发', async () => {
    const { client } = harness();
    const sessionIds = Array.from({ length: 25 }, (_, index) => session(index));
    client.crypto.keys = sessionIds.map((sessionId) => exported(sessionId));

    client.emit('receivedToDeviceMessage', delivered(request({ sessionIds })));

    await vi.waitFor(() => {
      expect(client.queueToDevice).toHaveBeenCalledTimes(2);
    });
    const sizes = client.crypto.encryptToDeviceMessages.mock.calls.map(
      ([, , payload]) => (payload as { keys: unknown[] }).keys.length,
    );
    expect(sizes).toEqual([20, 5]);
  });

  it.each(silentCases)('$label时一声不吭', async ({ message, prepare }) => {
    const { client } = harness();
    prepare?.(client);

    client.emit('receivedToDeviceMessage', message?.() ?? delivered(request()));
    await settle();

    expect(client.crypto.encryptToDeviceMessages).not.toHaveBeenCalled();
    expect(client.queueToDevice).not.toHaveBeenCalled();
  });

  it('同一台设备在同一个房间十分钟答一次', async () => {
    const { client, clock } = harness();
    client.emit('receivedToDeviceMessage', delivered(request()));
    await vi.waitFor(() => {
      expect(client.queueToDevice).toHaveBeenCalledOnce();
    });

    clock.now += 9 * 60 * 1_000;
    client.emit('receivedToDeviceMessage', delivered(request()));
    await settle();
    expect(client.queueToDevice).toHaveBeenCalledOnce();

    clock.now += 60 * 1_000;
    client.emit('receivedToDeviceMessage', delivered(request()));
    await vi.waitFor(() => {
      expect(client.queueToDevice).toHaveBeenCalledTimes(2);
    });
  });

  it('这台设备每分钟最多答 20 次，超出的连设备都不查', async () => {
    const { client, clock } = harness();
    for (let index = 0; index < 21; index += 1) {
      client.emit(
        'receivedToDeviceMessage',
        delivered(request(), { device: `UNKNOWN${String(index)}` }),
      );
    }
    await settle();
    // 前 20 个过了频率、卡在设备检查上（不认识这些设备），第 21 个在查设备之前就被挡下。
    expect(client.crypto.getUserDeviceInfo).toHaveBeenCalledTimes(20);

    client.emit('receivedToDeviceMessage', delivered(request()));
    await settle();
    expect(client.queueToDevice).not.toHaveBeenCalled();

    clock.now += 60 * 1_000;
    client.emit('receivedToDeviceMessage', delivered(request()));
    await vi.waitFor(() => {
      expect(client.queueToDevice).toHaveBeenCalledOnce();
    });
  });

  it('几台设备接连来请求时共用一次导出，过一会儿再来就重新导出', async () => {
    const { client, clock } = harness();
    client.emit('receivedToDeviceMessage', delivered(request()));
    client.emit('receivedToDeviceMessage', delivered(request(), { device: SECOND_DEVICE }));
    await vi.waitFor(() => {
      expect(client.queueToDevice).toHaveBeenCalledTimes(2);
    });
    expect(client.crypto.exportRoomKeys).toHaveBeenCalledOnce();

    clock.now += 11 * 60 * 1_000;
    client.emit('receivedToDeviceMessage', delivered(request()));
    await vi.waitFor(() => {
      expect(client.queueToDevice).toHaveBeenCalledTimes(3);
    });
    expect(client.crypto.exportRoomKeys).toHaveBeenCalledTimes(2);
  });

  it('换了账户就不再听旧客户端', () => {
    const { client, registry } = harness();

    registry.replace(null);

    expect(client.listeners('receivedToDeviceMessage')).toBe(0);
  });
});
