// @vitest-environment jsdom
import '@testing-library/jest-dom/vitest';
import { cleanup, render } from '@testing-library/react';
import { I18nextProvider } from 'react-i18next';
import { afterEach, describe, expect, it } from 'vitest';

import { i18n, initializeI18n } from '@/shared/i18n/i18n';

import { RoomKindLabel, roomKindWarns, type RoomKind } from './room-kind-label';

afterEach(cleanup);

function label(kind: RoomKind): HTMLElement {
  const view = render(
    <I18nextProvider i18n={i18n}>
      <RoomKindLabel {...kind} />
    </I18nextProvider>,
  );
  const element = view.container.firstElementChild;
  if (!(element instanceof HTMLElement)) throw new Error('没有画出那一小句。');
  return element;
}

const warnings: readonly (readonly [RoomKind, string, string])[] = [
  [{ privateRoom: true, archived: true }, 'Private room · Archived', '私人房间 · 已归档'],
  [
    { privateRoom: true, serverReadable: true },
    'Private room · The server can read it',
    '私人房间 · 服务器能读到',
  ],
  [
    { privateRoom: true, listed: true, unencrypted: true },
    'Public room · Not encrypted',
    '公开房间 · 不加密',
  ],
];

describe('房间名旁边那一小句', () => {
  for (const [language, column] of [
    ['en', 1],
    ['zh-CN', 2],
  ] as const) {
    it(`要一直提醒着的几句分成两半，前一半单独包着，地方不够时只收它（${language}）`, async () => {
      await initializeI18n(window.localStorage, [language]);
      for (const warning of warnings) {
        const [kind] = warning;
        const text = warning[column];
        expect(roomKindWarns(kind)).toBe(true);
        const element = label(kind);
        // 整句照旧（宽的时候、读屏读到的都是整句）。
        expect(element).toHaveTextContent(text);
        expect(element).toHaveClass('workspace-header__notice');
        const [what, notice] = text.split(' · ');
        const first = element.querySelector('.room-kind__what');
        expect(first?.textContent).toBe(`${what ?? ''} · `);
        // 后一半直接写在外面那一层里：前一半藏起来以后剩下的就是它。
        expect(element.lastChild?.textContent).toBe(notice);
        cleanup();
      }
    });
  }

  it('不用提醒的几种只有一句，不分两半', async () => {
    await initializeI18n(window.localStorage, ['en']);
    for (const kind of [
      { privateRoom: true },
      { privateRoom: true, listed: true },
      { privateRoom: false },
    ] satisfies RoomKind[]) {
      expect(roomKindWarns(kind)).toBe(false);
      expect(label(kind).querySelector('.room-kind__what')).toBeNull();
      cleanup();
    }
  });
});
