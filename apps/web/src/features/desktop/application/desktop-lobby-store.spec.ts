import { describe, expect, it, vi } from 'vitest';

import { DesktopLobbyStore } from '@/features/desktop/application/desktop-lobby-store';
import type {
  DesktopLobbySnapshot,
  DesktopRuntimeGateway,
} from '@/features/desktop/domain/desktop-runtime';
import { err, ok } from '@/shared/result';

describe('桌面大厅 Store', () => {
  it('首次订阅读取真实投影，失败后允许显式重试', async () => {
    const lobby = snapshot();
    const readLobby = vi
      .fn<DesktopRuntimeGateway['readLobby']>()
      .mockResolvedValueOnce(err({ code: 'bridge.ipc.timeout', retryable: true }))
      .mockResolvedValue(ok(lobby));
    const store = new DesktopLobbyStore({ readLobby } as unknown as DesktopRuntimeGateway);
    const listener = vi.fn();
    const unsubscribe = store.subscribe(listener);

    await vi.waitFor(() => {
      expect(store.getSnapshot()).toEqual({
        failure: { code: 'bridge.ipc.timeout', retryable: true },
        kind: 'failed',
      });
    });
    store.retry();
    await vi.waitFor(() => {
      expect(store.getSnapshot()).toEqual({ kind: 'ready', snapshot: lobby });
    });

    expect(readLobby).toHaveBeenCalledTimes(2);
    unsubscribe();
  });
});

function snapshot(): DesktopLobbySnapshot {
  return {
    agents: [],
    identity: {
      agent: {
        agentId: '0198b601-77a1-7bb8-83eb-a8fe68c97e44',
        avatarUrl: null,
        displayName: 'Agent',
        matrixUserId: '@agent:matrix.test',
      },
      connectionState: 'ready',
      grantedCapabilities: [],
      instanceId: '0198b601-77a4-7bb8-83eb-a8fe68c97e44',
      matrixDeviceId: 'DEVICE',
      roomId: '!public:matrix.test',
    },
    messages: [],
    nextCursor: null,
    observedAtUnixMs: 1_200,
  };
}
