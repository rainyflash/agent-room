import { describe, expect, it, vi } from 'vitest';

import {
  BrowserWebNotifications,
  webNotificationsActive,
  type NotificationApi,
} from './web-notifications';

type Created = { title: string; options: unknown; onclick: ((event: Event) => void) | null };

function fakeApi(permission: string, answer = 'granted') {
  const created: Created[] = [];
  const close = vi.fn();
  const api = vi.fn(function (this: Created, title: string, options: unknown) {
    this.title = title;
    this.options = options;
    this.onclick = null;
    Object.assign(this, { close });
    created.push(this);
  }) as unknown as NotificationApi & { permission: string };
  const requestPermission = vi.fn(() => {
    api.permission = answer;
    return Promise.resolve(answer);
  });
  Object.assign(api, { permission, requestPermission });
  return { api, close, created, requestPermission };
}

function memoryStorage(initial: Record<string, string> = {}) {
  const values = new Map(Object.entries(initial));
  return {
    getItem: (key: string) => values.get(key) ?? null,
    setItem: (key: string, value: string) => {
      values.set(key, value);
    },
  };
}

const note = (onClick: () => void = () => undefined) => ({
  body: 'Lin: can you look at the migration?',
  onClick,
  tag: 'agent-room-room:Backups',
  title: 'Backups',
});

describe('BrowserWebNotifications', () => {
  it('没有 Notification 的浏览器算不支持，打开了也不弹', async () => {
    const notifications = new BrowserWebNotifications(null, memoryStorage());
    expect(notifications.state()).toEqual({
      offered: false,
      permission: 'unsupported',
      wanted: false,
    });
    const state = await notifications.setWanted(true);
    expect(webNotificationsActive(state)).toBe(false);
    expect(notifications.show(note())).toBe(false);
  });

  it('打开时浏览器还没问过就先问，给了就生效，并通知订阅的人', async () => {
    const { api, requestPermission } = fakeApi('default');
    const storage = memoryStorage();
    const notifications = new BrowserWebNotifications(api, storage);
    const listener = vi.fn();
    notifications.subscribe(listener);

    const state = await notifications.setWanted(true);
    expect(requestPermission).toHaveBeenCalledTimes(1);
    expect(state).toEqual({ offered: false, permission: 'granted', wanted: true });
    expect(listener).toHaveBeenCalled();
    // 记在这个浏览器里，下次打开还在。
    expect(new BrowserWebNotifications(api, storage).state().wanted).toBe(true);
  });

  it('浏览器拒过的不再问，开关记着但不生效', async () => {
    const { api, requestPermission } = fakeApi('denied');
    const notifications = new BrowserWebNotifications(api, memoryStorage());
    const state = await notifications.setWanted(true);
    expect(requestPermission).not.toHaveBeenCalled();
    expect(state).toEqual({ offered: false, permission: 'denied', wanted: true });
    expect(webNotificationsActive(state)).toBe(false);
  });

  it('开着、给了权限才弹；点了关掉通知、回到那一条', () => {
    const { api, close, created } = fakeApi('granted');
    const off = new BrowserWebNotifications(api, memoryStorage());
    expect(off.show(note())).toBe(false);
    expect(created).toHaveLength(0);

    const notifications = new BrowserWebNotifications(
      api,
      memoryStorage({ 'agent-room.web-notifications.v1': 'on' }),
    );
    const clicked = vi.fn();
    expect(notifications.show(note(clicked))).toBe(true);
    expect(created).toHaveLength(1);
    expect(created[0]).toMatchObject({
      options: {
        body: 'Lin: can you look at the migration?',
        icon: '/icons/192x192.png',
        tag: 'agent-room-room:Backups',
      },
      title: 'Backups',
    });
    created[0]?.onclick?.(new Event('click'));
    expect(close).toHaveBeenCalledTimes(1);
    expect(clicked).toHaveBeenCalledTimes(1);
  });

  it('浏览器不让直接弹（安卓上的 Chrome）时回答没弹', () => {
    const api = vi.fn(() => {
      throw new TypeError('Illegal constructor');
    }) as unknown as NotificationApi;
    Object.assign(api, { permission: 'granted', requestPermission: vi.fn() });
    const notifications = new BrowserWebNotifications(
      api,
      memoryStorage({ 'agent-room.web-notifications.v1': 'on' }),
    );
    expect(notifications.show(note())).toBe(false);
  });

  it('问过一次记下；不让存的浏览器照样能用', () => {
    const { api } = fakeApi('default');
    const storage = memoryStorage();
    const notifications = new BrowserWebNotifications(api, storage);
    notifications.markOffered();
    expect(notifications.state().offered).toBe(true);
    expect(new BrowserWebNotifications(api, storage).state().offered).toBe(true);

    const broken = {
      getItem: () => {
        throw new Error('SecurityError');
      },
      setItem: () => {
        throw new Error('SecurityError');
      },
    };
    const plain = new BrowserWebNotifications(api, broken);
    expect(plain.state()).toEqual({ offered: false, permission: 'default', wanted: false });
    expect(() => {
      plain.markOffered();
    }).not.toThrow();
  });

  it('人在浏览器设置里改了权限，回到页面重读时跟着变', () => {
    const { api } = fakeApi('granted');
    const notifications = new BrowserWebNotifications(
      api,
      memoryStorage({ 'agent-room.web-notifications.v1': 'on' }),
    );
    const listener = vi.fn();
    notifications.subscribe(listener);
    expect(webNotificationsActive(notifications.state())).toBe(true);
    (api as { permission: string }).permission = 'denied';
    notifications.refresh();
    expect(notifications.state().permission).toBe('denied');
    expect(listener).toHaveBeenCalledTimes(1);
    // 没变就不叫。
    notifications.refresh();
    expect(listener).toHaveBeenCalledTimes(1);
  });
});
