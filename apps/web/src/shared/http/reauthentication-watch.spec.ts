import { describe, expect, it, vi } from 'vitest';

import { ReauthenticationWatch } from './reauthentication-watch';

function answering(status: number, body: string): typeof globalThis.fetch {
  return () =>
    Promise.resolve(
      new Response(body, { status, headers: { 'Content-Type': 'application/json' } }),
    );
}

async function settle(): Promise<void> {
  await new Promise((resolve) => setTimeout(resolve, 0));
}

describe('服务器说要重新登录', () => {
  it('只在 401 回“要最近登录过”时通知，调用方照常读到原来的回答', async () => {
    const body = JSON.stringify({ code: 'authentication.reauthentication_required' });
    const watch = new ReauthenticationWatch(answering(401, body));
    const listener = vi.fn();
    watch.subscribe(listener);

    const response = await watch.fetch('https://api.agent-room.test/account', {
      method: 'DELETE',
    });
    await settle();

    expect(listener).toHaveBeenCalledTimes(1);
    await expect(response.json()).resolves.toEqual({
      code: 'authentication.reauthentication_required',
    });
  });

  it.each([
    [401, JSON.stringify({ code: 'authentication.invalid_session' })],
    [401, 'not json'],
    [403, JSON.stringify({ code: 'authentication.reauthentication_required' })],
    [200, JSON.stringify({ ok: true })],
  ])('%i %s 不算', async (status, body) => {
    const watch = new ReauthenticationWatch(answering(status, body));
    const listener = vi.fn();
    watch.subscribe(listener);

    await watch.fetch('https://api.agent-room.test/account');
    await settle();

    expect(listener).not.toHaveBeenCalled();
  });

  it('退订以后不再通知', async () => {
    const body = JSON.stringify({ code: 'authentication.reauthentication_required' });
    const watch = new ReauthenticationWatch(answering(401, body));
    const listener = vi.fn();
    watch.subscribe(listener)();

    await watch.fetch('https://api.agent-room.test/account');
    await settle();

    expect(listener).not.toHaveBeenCalled();
  });
});
