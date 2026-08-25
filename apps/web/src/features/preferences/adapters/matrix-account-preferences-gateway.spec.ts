import { ClientEvent, type MatrixClient, type MatrixEvent } from 'matrix-js-sdk';
import { describe, expect, it, vi } from 'vitest';

import {
  ACCOUNT_PREFERENCES_EVENT_TYPE,
  MatrixAccountPreferencesGateway,
} from './matrix-account-preferences-gateway';
import { createAccountPreferencesDocument } from '@/features/preferences/domain/account-preferences';
import { MatrixClientRegistry } from '@/shared/matrix/matrix-client-registry';

describe('Matrix 账户偏好网关', () => {
  it('没有完整 Matrix 账户作用域时失败关闭', async () => {
    const gateway = new MatrixAccountPreferencesGateway(new MatrixClientRegistry());

    await expect(gateway.read()).resolves.toEqual({
      error: { code: 'preferences.source_unavailable', retryable: true },
      ok: false,
    });
    await expect(gateway.write(document())).resolves.toEqual({
      error: { code: 'preferences.source_unavailable', retryable: true },
      ok: false,
    });
  });

  it('严格解析服务端事件，并把 I/O 异常映射为可重试失败', async () => {
    const registry = new MatrixClientRegistry();
    const client = matrixClient(document());
    registry.replace(client.value);
    const gateway = new MatrixAccountPreferencesGateway(registry);

    await expect(gateway.read()).resolves.toEqual({ ok: true, value: document() });
    client.read.mockResolvedValueOnce({ ...document(), unknown: true });
    await expect(gateway.read()).resolves.toEqual({
      error: { code: 'preferences.invalid_document', retryable: false },
      ok: false,
    });
    client.read.mockRejectedValueOnce(new Error('offline'));
    await expect(gateway.read()).resolves.toEqual({
      error: { code: 'preferences.read_failed', retryable: true },
      ok: false,
    });
    client.write.mockRejectedValueOnce(new Error('offline'));
    await expect(gateway.write(document())).resolves.toEqual({
      error: { code: 'preferences.write_failed', retryable: true },
      ok: false,
    });
  });

  it('只响应目标 Account Data，并在 Matrix 客户端切换时解绑旧监听', () => {
    const registry = new MatrixClientRegistry();
    const first = matrixClient(document());
    const second = matrixClient(document(), '@other:agent-room.test', 'DEVICE_B');
    registry.replace(first.value);
    const gateway = new MatrixAccountPreferencesGateway(registry);
    const listener = vi.fn();
    const unsubscribe = gateway.subscribe(listener);

    first.emit('m.direct');
    first.emit(ACCOUNT_PREFERENCES_EVENT_TYPE);
    registry.replace(second.value);
    first.emit(ACCOUNT_PREFERENCES_EVENT_TYPE);
    second.emit(ACCOUNT_PREFERENCES_EVENT_TYPE);
    unsubscribe();
    second.emit(ACCOUNT_PREFERENCES_EVENT_TYPE);

    expect(listener).toHaveBeenCalledTimes(3);
    expect(first.removeListener).toHaveBeenCalledWith(
      ClientEvent.AccountData,
      expect.any(Function),
    );
    expect(second.removeListener).toHaveBeenCalledWith(
      ClientEvent.AccountData,
      expect.any(Function),
    );
    expect(gateway.scope()).toEqual({
      accountId: '@other:agent-room.test',
      writerId: 'DEVICE_B',
    });
  });
});

function document() {
  const result = createAccountPreferencesDocument(
    { language: 'system', lobbyView: 'scene' },
    'DEVICE_A',
  );
  if (!result.ok) {
    throw new Error('测试文档创建失败。');
  }
  return result.value;
}

function matrixClient(
  initialContent: unknown,
  userId = '@operator:agent-room.test',
  deviceId = 'DEVICE_A',
) {
  const accountDataListeners = new Set<(event: MatrixEvent) => void>();
  const read = vi.fn<() => Promise<unknown>>().mockResolvedValue(initialContent);
  const write = vi.fn<() => Promise<unknown>>().mockResolvedValue({});
  const on = vi.fn((event: ClientEvent, listener: (matrixEvent: MatrixEvent) => void) => {
    if (event === ClientEvent.AccountData) {
      accountDataListeners.add(listener);
    }
  });
  const removeListener = vi.fn(
    (event: ClientEvent, listener: (matrixEvent: MatrixEvent) => void) => {
      if (event === ClientEvent.AccountData) {
        accountDataListeners.delete(listener);
      }
    },
  );
  const value = {
    getAccountDataFromServer: read,
    getDeviceId: () => deviceId,
    getUserId: () => userId,
    on,
    removeListener,
    setAccountData: write,
  } as unknown as MatrixClient;
  return {
    emit: (type: string) => {
      const event = { getType: () => type } as unknown as MatrixEvent;
      for (const listener of accountDataListeners) {
        listener(event);
      }
    },
    on,
    read,
    removeListener,
    value,
    write,
  };
}
