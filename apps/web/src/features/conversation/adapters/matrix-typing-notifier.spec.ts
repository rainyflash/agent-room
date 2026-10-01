import type { MatrixClient } from 'matrix-js-sdk';
import { describe, expect, it, vi } from 'vitest';

import { MatrixTypingNotifier } from './matrix-typing-notifier';
import type { MatrixClientSource } from '@/shared/matrix/matrix-client-registry';

const roomId = '!lobby:agent-room.test';

function source(
  membership: string | null,
  sendTyping: MatrixClient['sendTyping'],
): MatrixClientSource {
  const client = {
    getRoom: (id: string) =>
      id === roomId && membership !== null ? { getMyMembership: () => membership } : null,
    sendTyping,
  } as unknown as MatrixClient;
  return { current: () => client, subscribe: () => () => undefined };
}

describe('MatrixTypingNotifier', () => {
  it('在房间里才说，请服务器最多算 30 秒，发不出去也不抛错', async () => {
    const sendTyping = vi.fn().mockRejectedValue(new Error('offline'));
    new MatrixTypingNotifier(source('join', sendTyping)).typing(roomId, true);
    expect(sendTyping).toHaveBeenCalledWith(roomId, true, 30_000);
    await Promise.resolve();

    const outside = vi.fn().mockResolvedValue({});
    new MatrixTypingNotifier(source('leave', outside)).typing(roomId, true);
    new MatrixTypingNotifier(source(null, outside)).typing(roomId, false);
    expect(outside).not.toHaveBeenCalled();
  });

  it('没有登录时什么也不做', () => {
    const notifier = new MatrixTypingNotifier({
      current: () => null,
      subscribe: () => () => undefined,
    });
    expect(() => {
      notifier.typing(roomId, true);
    }).not.toThrow();
  });
});
