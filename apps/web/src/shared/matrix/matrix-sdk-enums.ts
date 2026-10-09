import type {
  ClientEvent,
  Direction,
  EventStatus,
  HistoryVisibility,
  MatrixEventEvent,
  SyncState,
} from 'matrix-js-sdk';
import type { CryptoEvent } from 'matrix-js-sdk/lib/crypto-api/index.js';

/**
 * matrix-js-sdk 字符串枚举的取值。
 *
 * 枚举是运行时值，静态导入它们会把整个 SDK 拉进首屏主包，抵消会话网关里刻意的动态导入；
 * 构建器会以 INEFFECTIVE_DYNAMIC_IMPORT 报出这一点。这里只固定取值本身：模板字面量类型把
 * 枚举展开成它的字符串取值集合，因此拼写错误或 SDK 改名都会在编译期失败，而不是运行时才发现。
 */
const enumValue = <Enum extends string>(value: `${Enum}`): Enum => value as Enum;

export const EVENT_STATUS_SENT = enumValue<EventStatus>('sent');
export const EVENT_STATUS_NOT_SENT = enumValue<EventStatus>('not_sent');

export const DIRECTION_FORWARD = enumValue<Direction>('f');
export const DIRECTION_BACKWARD = enumValue<Direction>('b');

export const SYNC_STATE_PREPARED = enumValue<SyncState>('PREPARED');
export const SYNC_STATE_SYNCING = enumValue<SyncState>('SYNCING');
export const SYNC_STATE_ERROR = enumValue<SyncState>('ERROR');
export const SYNC_STATE_RECONNECTING = enumValue<SyncState>('RECONNECTING');

export const CLIENT_EVENT_ACCOUNT_DATA = enumValue<ClientEvent>('accountData');
export const CLIENT_EVENT_EVENT = enumValue<ClientEvent.Event>('event');
export const CLIENT_EVENT_SYNC = enumValue<ClientEvent>('sync');
export const CLIENT_EVENT_RECEIVED_TO_DEVICE_MESSAGE =
  enumValue<ClientEvent.ReceivedToDeviceMessage>('receivedToDeviceMessage');

export const MATRIX_EVENT_DECRYPTED = enumValue<MatrixEventEvent.Decrypted>('Event.decrypted');

export const HISTORY_VISIBILITY_SHARED = enumValue<HistoryVisibility>('shared');
export const HISTORY_VISIBILITY_WORLD_READABLE = enumValue<HistoryVisibility>('world_readable');

export const CRYPTO_EVENT_DEVICES_UPDATED =
  enumValue<CryptoEvent.DevicesUpdated>('crypto.devicesUpdated');
export const CRYPTO_EVENT_USER_TRUST_STATUS_CHANGED =
  enumValue<CryptoEvent.UserTrustStatusChanged>('userTrustStatusChanged');
