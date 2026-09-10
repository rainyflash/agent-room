// @vitest-environment jsdom

import { afterEach, describe, expect, it, vi } from 'vitest';

import { safeInternalPath, WindowBrowserGateway } from './window-browser-gateway';

afterEach(() => {
  vi.unstubAllGlobals();
});

describe('WindowBrowserGateway', () => {
  it('验证失败标记不能被返回房间的深链覆盖', () => {
    window.history.replaceState({}, '', '/connect?authentication=expired&returnTo=%2Frooms');
    expect(new WindowBrowserGateway().currentPath()).toBe(
      '/connect?authentication=expired&returnTo=%2Frooms',
    );
  });
  it('连接后只导航到新地址，恢复当前房间时不重新加载页面', () => {
    const location = { pathname: '/connect', search: '', hash: '', replace: vi.fn() };
    vi.stubGlobal('window', { location });
    const browser = new WindowBrowserGateway();
    browser.replacePath('/rooms');
    expect(location.replace).toHaveBeenCalledExactlyOnceWith('/rooms');
    location.pathname = '/rooms';
    browser.replacePath('/rooms');
    expect(location.replace).toHaveBeenCalledOnce();
    browser.replacePath('/rooms?directory=open');
    expect(location.replace).toHaveBeenLastCalledWith('/rooms?directory=open');
  });

  it.each([
    ['/lobby/public?directory=open', '/lobby/public?directory=open'],
    ['//evil.example/path', null],
    ['/safe\\escape', null],
    ['https://evil.example', null],
    [null, null],
  ])('只接受同源内部回跳 %s', (candidate, expected) => {
    expect(safeInternalPath(candidate)).toBe(expected);
  });

  it('在连接页优先恢复显式深链并移除 Matrix 单次 Token', () => {
    window.history.replaceState(
      {},
      '',
      '/connect?returnTo=%2Flobby%2Fpublic%3Fdirectory%3Dopen&loginToken=secret',
    );

    expect(new WindowBrowserGateway().currentPath()).toBe('/lobby/public?directory=open');
  });
});
