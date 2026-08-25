import type { MatrixClient } from 'matrix-js-sdk';
import { describe, expect, it, vi } from 'vitest';

import { MatrixSdkPrivateRoomGateway } from './matrix-private-room-gateway';
import type { MatrixClientSource } from '@/shared/matrix/matrix-client-registry';

describe('MatrixSdkPrivateRoomGateway', () => {
  it('没有同步客户端时明确失败而不伪装加入', async () => {
    const gateway = new MatrixSdkPrivateRoomGateway(source(null));

    await expect(gateway.join('!private:matrix.test')).resolves.toEqual({
      error: { code: 'private_room.matrix_unavailable', retryable: true },
      ok: false,
    });
  });

  it('只在 Matrix joinRoom 真正完成后报告成功', async () => {
    const joinRoom = vi.fn().mockResolvedValue({});
    const gateway = new MatrixSdkPrivateRoomGateway(
      source({ joinRoom } as unknown as MatrixClient),
    );

    const result = await gateway.join('!private:matrix.test');

    expect(result).toEqual({ ok: true, value: undefined });
    expect(joinRoom).toHaveBeenCalledWith('!private:matrix.test');
  });
});

function source(client: MatrixClient | null): MatrixClientSource {
  return {
    current: () => client,
    subscribe: () => () => undefined,
  };
}
