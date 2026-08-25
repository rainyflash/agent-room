import { EventType, Preset, Visibility, type MatrixClient } from 'matrix-js-sdk';
import { z } from 'zod';

import type { MatrixClientSource } from '@/shared/matrix/matrix-client-registry';

const driverProperty = '__agentRoomVerticalSecurityDriver' as const;
const recoverySampleEventType = 'org.agentroom.test.recovery_sample.v1';
const megolmAlgorithm = 'm.megolm.v1.aes-sha2';
const waitStepMilliseconds = 250;
const roomPreparationTimeoutMilliseconds = 20_000;
const keyBackupTimeoutMilliseconds = 30_000;

declare module 'matrix-js-sdk' {
  interface TimelineEvents {
    [recoverySampleEventType]: {
      readonly nonce: string;
    };
  }
}

const recoverySampleContentSchema = z
  .object({
    nonce: z.uuid(),
  })
  .strict();

export type VerticalSecuritySample = {
  readonly eventId: string;
  readonly nonce: string;
  readonly roomId: string;
};

export type VerticalSecurityDriver = {
  createRecoverySample(): Promise<VerticalSecuritySample>;
  decryptRecoverySample(sample: VerticalSecuritySample): Promise<void>;
};

declare global {
  interface Window {
    __agentRoomVerticalSecurityDriver?: VerticalSecurityDriver;
  }
}

export function installVerticalSecurityDriver(clients: MatrixClientSource): () => void {
  const driver = createVerticalSecurityDriver(clients);
  Object.defineProperty(window, driverProperty, {
    configurable: true,
    value: driver,
  });
  return () => {
    if (window[driverProperty] === driver) {
      Reflect.deleteProperty(window, driverProperty);
    }
  };
}

export function createVerticalSecurityDriver(clients: MatrixClientSource): VerticalSecurityDriver {
  return Object.freeze({
    createRecoverySample: async () => await createRecoverySample(requireClient(clients)),
    decryptRecoverySample: async (sample) => {
      await decryptRecoverySample(requireClient(clients), sample);
    },
  });
}

async function createRecoverySample(client: MatrixClient): Promise<VerticalSecuritySample> {
  const crypto = requireCrypto(client);
  if ((await crypto.getActiveSessionBackupVersion()) === null) {
    throw new Error('纵向安全驱动拒绝在密钥备份未启用时创建恢复样本。');
  }
  const initialBackup = await crypto.checkKeyBackupAndEnable();
  if (initialBackup === null) {
    throw new Error('纵向安全驱动没有找到可信的活动密钥备份。');
  }

  const created = await client.createRoom({
    initial_state: [
      {
        content: { algorithm: megolmAlgorithm },
        state_key: '',
        type: EventType.RoomEncryption,
      },
    ],
    name: 'Agent Room encrypted recovery verification',
    preset: Preset.PrivateChat,
    visibility: Visibility.Private,
  });
  await waitUntil(
    async () =>
      client.getRoom(created.room_id)?.hasEncryptionStateEvent() === true &&
      (await crypto.isEncryptionEnabledInRoom(created.room_id)),
    roomPreparationTimeoutMilliseconds,
    '加密恢复样本房间没有进入可安全发送状态。',
  );

  const nonce = cryptoRandomUuid();
  const sent = await client.sendEvent(created.room_id, recoverySampleEventType, { nonce });
  const rawEvent = await client.fetchRoomEvent(created.room_id, sent.event_id);
  if (rawEvent.type !== EventType.RoomMessageEncrypted) {
    throw new Error('恢复样本被明文写入 Matrix，纵向验收立即失败。');
  }

  await waitUntil(
    async () => {
      const current = await crypto.checkKeyBackupAndEnable();
      return current !== null && current.backupInfo.count > initialBackup.backupInfo.count;
    },
    keyBackupTimeoutMilliseconds,
    '恢复样本的 Megolm 会话密钥没有上传到服务端备份。',
  );

  return Object.freeze({ eventId: sent.event_id, nonce, roomId: created.room_id });
}

async function decryptRecoverySample(
  client: MatrixClient,
  sample: VerticalSecuritySample,
): Promise<void> {
  requireCrypto(client);
  const rawEvent = await client.fetchRoomEvent(sample.roomId, sample.eventId);
  if (rawEvent.type !== EventType.RoomMessageEncrypted) {
    throw new Error('恢复验证读取到的不是 Matrix 加密事件。');
  }
  const event = client.getEventMapper({ decrypt: false, preventReEmit: true })(rawEvent);
  await client.decryptEventIfNeeded(event);
  if (event.isDecryptionFailure() || event.getType() !== recoverySampleEventType) {
    throw new Error('新设备无法使用恢复后的房间密钥解密验证事件。');
  }
  const content = recoverySampleContentSchema.safeParse(event.getContent());
  if (!content.success || content.data.nonce !== sample.nonce) {
    throw new Error('恢复验证事件的明文挑战与原始样本不一致。');
  }
}

function requireClient(clients: MatrixClientSource): MatrixClient {
  const client = clients.current();
  if (client === null) {
    throw new Error('纵向安全驱动没有可用的 Matrix 客户端。');
  }
  return client;
}

function requireCrypto(client: MatrixClient) {
  const crypto = client.getCrypto();
  if (crypto === undefined) {
    throw new Error('纵向安全驱动没有可用的 Matrix 加密实现。');
  }
  return crypto;
}

async function waitUntil(
  predicate: () => Promise<boolean>,
  timeoutMilliseconds: number,
  failureMessage: string,
): Promise<void> {
  const deadline = Date.now() + timeoutMilliseconds;
  while (Date.now() < deadline) {
    if (await predicate()) {
      return;
    }
    await new Promise((resolve) => window.setTimeout(resolve, waitStepMilliseconds));
  }
  throw new Error(failureMessage);
}

function cryptoRandomUuid(): string {
  if (typeof window.crypto.randomUUID !== 'function') {
    throw new Error('当前浏览器不能生成恢复样本的密码学随机标识。');
  }
  return window.crypto.randomUUID();
}
