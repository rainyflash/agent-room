import { describe, expect, it, vi } from 'vitest';
import { LobbyExperienceStore } from './lobby-experience-store';
import { RoomActivityStore } from './room-activity-store';
import {
  presenceAgent,
  presenceMessage,
  presenceRoom,
  selfIdentity,
} from '../testing/room-presence-fixtures';
import type { RoomMessageSignal } from '@/features/messages/domain/message';
import { ok } from '@/shared/result';

describe('大厅组合状态', () => {
  it('房间、气泡和聊天共用一个消息源，成员更新保留位置并清理所有订阅', () => {
    let room = presenceRoom();
    let messages: readonly RoomMessageSignal[] = [];
    let notifyLobby = (): void => undefined;
    let notifyMessages = (): void => undefined;
    const detachLobby = vi.fn();
    const detachMessages = vi.fn();
    const subscribeMessages = vi.fn((_roomId: string, listener: () => void) => {
      notifyMessages = listener;
      return detachMessages;
    });
    const store = new LobbyExperienceStore(
      {
        read: () => ok(room),
        subscribe: (_roomId, listener) => {
          notifyLobby = listener;
          return detachLobby;
        },
      },
      {
        read: (roomId) =>
          ok({ roomId, messages, readOnlyFederatedEvents: [], observedAtUnixMs: Date.now() }),
        subscribe: subscribeMessages,
      },
      room.roomId,
      selfIdentity,
    );
    const stopLobby = store.subscribe(() => undefined);
    const stopActivity = new RoomActivityStore(store.messages, selfIdentity.matrixUserId).subscribe(
      () => undefined,
    );
    const stopChat = store.messages.subscribe(() => undefined);
    const initial = store.getSnapshot();
    expect(initial.kind).toBe('ready');
    if (initial.kind !== 'ready') throw new Error('大厅未加载');
    room = { ...room, agents: [...room.agents, presenceAgent(24)] };
    notifyLobby();
    messages = [presenceMessage('guest')];
    notifyMessages();
    const next = store.getSnapshot();
    if (next.kind !== 'ready') throw new Error('大厅未加载');
    expect(next.projection.humans).toHaveLength(2);
    for (const [id, point] of initial.projection.layout ?? [])
      expect(next.projection.layout?.get(id)).toEqual(point);
    expect(subscribeMessages).toHaveBeenCalledOnce();
    stopLobby();
    stopActivity();
    expect(detachMessages).not.toHaveBeenCalled();
    stopChat();
    expect(detachLobby).toHaveBeenCalledOnce();
    expect(detachMessages).toHaveBeenCalledOnce();
  });
});
