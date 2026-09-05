import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { RoomActivityStore, roomSpeechLifetimeMs } from './room-activity-store';
import { MessageRoomStore } from '@/features/messages/application/message-room-store';
import type { RoomMessageSignal } from '@/features/messages/domain/message';
import { ok } from '@/shared/result';
import {
  guestActor,
  presenceMessage,
  presenceTime,
  selfIdentity,
} from '../testing/room-presence-fixtures';

describe('房间发言与未读生命周期', () => {
  beforeEach(() => {
    vi.useFakeTimers();
    vi.setSystemTime(presenceTime);
  });
  afterEach(() => {
    vi.useRealTimers();
  });

  it('历史加载不标未读，新增他人公开发言计数，打开大厅对话才清除', () => {
    const fixture = activityFixture([presenceMessage('history')]);
    const detach = fixture.activity.subscribe(() => undefined);
    expect(fixture.activity.getSnapshot().unread).toBe(0);
    fixture.publish([
      presenceMessage('history'),
      presenceMessage('incoming'),
      presenceMessage('own', { actor: { ...guestActor, ...selfIdentity } }),
      presenceMessage('private', { roomId: '!private:room.test' }),
      presenceMessage('resource', { preview: null }),
    ]);
    expect(fixture.activity.getSnapshot().unread).toBe(1);
    expect(fixture.activity.getSnapshot().recent.map((message) => message.messageId)).toEqual([
      'history',
      'incoming',
      'own',
    ]);
    fixture.activity.setVisible(true);
    expect(fixture.activity.getSnapshot().unread).toBe(0);
    fixture.publish([presenceMessage('visible')]);
    expect(fixture.activity.getSnapshot().unread).toBe(0);
    fixture.activity.setVisible(false);
    fixture.publish([presenceMessage('visible'), presenceMessage('next')]);
    expect(fixture.activity.getSnapshot().unread).toBe(1);
    detach();
  });

  it('气泡到期自动消失但保留未读，撤回消息同步移除且释放定时器', () => {
    const fixture = activityFixture([]);
    const detach = fixture.activity.subscribe(() => undefined);
    fixture.publish([presenceMessage('incoming')]);
    vi.advanceTimersByTime(roomSpeechLifetimeMs);
    expect(fixture.activity.getSnapshot()).toEqual({ recent: [], unread: 1 });
    fixture.publish([presenceMessage('incoming', { lifecycle: 'redacted' })]);
    expect(fixture.activity.getSnapshot()).toEqual({ recent: [], unread: 0 });
    fixture.publish([presenceMessage('another', { serverTimestamp: Date.now() })]);
    expect(vi.getTimerCount()).toBe(1);
    detach();
    expect(vi.getTimerCount()).toBe(0);
    expect(fixture.detachSource).toHaveBeenCalledOnce();
  });

  it('编辑不重复计数，补入旧历史不冒充新消息', () => {
    const fixture = activityFixture([]);
    const detach = fixture.activity.subscribe(() => undefined);
    fixture.publish([presenceMessage('incoming')]);
    fixture.publish([
      presenceMessage('incoming', { edited: true }),
      presenceMessage('old', { serverTimestamp: presenceTime - 60_000 }),
    ]);
    expect(fixture.activity.getSnapshot().unread).toBe(1);
    expect(fixture.activity.getSnapshot().recent).toHaveLength(1);
    detach();
  });
});

function activityFixture(initial: readonly RoomMessageSignal[]) {
  let messages = initial;
  let notify = (): void => undefined;
  const detachSource = vi.fn();
  const source = new MessageRoomStore(
    {
      read: (roomId) =>
        ok({ roomId, messages, readOnlyFederatedEvents: [], observedAtUnixMs: Date.now() }),
      subscribe: (_roomId, listener) => {
        notify = listener;
        return detachSource;
      },
    },
    '!lobby:room.test',
  );
  return {
    activity: new RoomActivityStore(source, selfIdentity.matrixUserId),
    detachSource,
    publish: (next: readonly RoomMessageSignal[]) => {
      messages = next;
      notify();
    },
  };
}
