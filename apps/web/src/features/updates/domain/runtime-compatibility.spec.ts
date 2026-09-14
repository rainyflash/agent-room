import { describe, expect, it } from 'vitest';

import { runtimeWriteAvailability } from './runtime-compatibility';

describe('PWA 运行时写入兼容门', () => {
  it('相同协议的界面更新允许继续发送，协议未知或改变仍暂停', () => {
    expect(
      runtimeWriteAvailability({ online: true, updateWaiting: true, contractCompatible: true })
        .allowed,
    ).toBe(true);
    expect(
      runtimeWriteAvailability({ online: true, updateWaiting: true, contractCompatible: false })
        .allowed,
    ).toBe(false);
    expect(
      runtimeWriteAvailability({ online: false, updateWaiting: true, contractCompatible: true }),
    ).toEqual({ allowed: false, reason: 'offline' });
  });
  it('在线且没有待激活版本时允许写入', () => {
    expect(runtimeWriteAvailability({ online: true, updateWaiting: false })).toEqual({
      allowed: true,
      reason: null,
    });
  });

  it('待激活版本优先于网络状态并禁止旧协议写入', () => {
    expect(runtimeWriteAvailability({ online: false, updateWaiting: true })).toEqual({
      allowed: false,
      reason: 'update_required',
    });
  });

  it('离线时不把请求交给 Service Worker 猜测重放', () => {
    expect(runtimeWriteAvailability({ online: false, updateWaiting: false })).toEqual({
      allowed: false,
      reason: 'offline',
    });
  });
});
