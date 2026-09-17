import { ClientEvent, Direction, EventStatus, SyncState } from 'matrix-js-sdk';
import { describe, expect, it } from 'vitest';

import {
  CLIENT_EVENT_ACCOUNT_DATA,
  CLIENT_EVENT_SYNC,
  DIRECTION_BACKWARD,
  DIRECTION_FORWARD,
  EVENT_STATUS_NOT_SENT,
  EVENT_STATUS_SENT,
  SYNC_STATE_ERROR,
  SYNC_STATE_PREPARED,
  SYNC_STATE_RECONNECTING,
  SYNC_STATE_SYNCING,
} from './matrix-sdk-enums';

// 这里是唯一允许在运行时导入 matrix-js-sdk 的位置：只有测试需要把固定取值与 SDK 对照。
describe('matrix-sdk-enums', () => {
  it('固定取值与 SDK 枚举逐一相等', () => {
    expect(EVENT_STATUS_SENT).toBe(EventStatus.SENT);
    expect(EVENT_STATUS_NOT_SENT).toBe(EventStatus.NOT_SENT);
    expect(DIRECTION_FORWARD).toBe(Direction.Forward);
    expect(DIRECTION_BACKWARD).toBe(Direction.Backward);
    expect(SYNC_STATE_PREPARED).toBe(SyncState.Prepared);
    expect(SYNC_STATE_SYNCING).toBe(SyncState.Syncing);
    expect(SYNC_STATE_ERROR).toBe(SyncState.Error);
    expect(SYNC_STATE_RECONNECTING).toBe(SyncState.Reconnecting);
    expect(CLIENT_EVENT_ACCOUNT_DATA).toBe(ClientEvent.AccountData);
    expect(CLIENT_EVENT_SYNC).toBe(ClientEvent.Sync);
  });
});
