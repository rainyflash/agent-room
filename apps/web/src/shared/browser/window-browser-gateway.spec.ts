// @vitest-environment jsdom

import { describe, expect, it } from 'vitest';

import { safeInternalPath, WindowBrowserGateway } from './window-browser-gateway';

describe('WindowBrowserGateway', () => {
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
