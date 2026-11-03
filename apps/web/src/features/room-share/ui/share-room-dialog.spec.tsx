// @vitest-environment jsdom

import '@testing-library/jest-dom/vitest';

import { cleanup, fireEvent, render, screen } from '@testing-library/react';
import { I18nextProvider } from 'react-i18next';
import { afterEach, beforeAll, beforeEach, describe, expect, it, vi } from 'vitest';

import { ShareRoomDialog } from '@/features/room-share/ui/share-room-dialog';
import { i18n, initializeI18n } from '@/shared/i18n/i18n';

const room = {
  description: 'Bring a crate error and we read it together.',
  name: 'Rust help desk',
  shareUrl: 'https://agentroom.test/r/aB3xYz',
};

beforeAll(async () => {
  await initializeI18n(window.localStorage, ['en']);
});

beforeEach(async () => {
  window.localStorage.clear();
  await i18n.changeLanguage('en');
});

afterEach(() => {
  cleanup();
  Reflect.deleteProperty(window.navigator, 'share');
});

function renderDialog() {
  return render(
    <I18nextProvider i18n={i18n}>
      <ShareRoomDialog onClose={() => undefined} room={room} />
    </I18nextProvider>,
  );
}

describe('分享对话框', () => {
  it('链接、默认的一段话、三家的分享网址、二维码和预览', () => {
    renderDialog();
    expect(screen.getByRole('textbox', { name: 'Link' })).toHaveValue(room.shareUrl);
    const message = screen.getByRole('textbox', { name: 'What you’ll say' });
    expect(message).toHaveValue(
      `Come chat in “Rust help desk”: Bring a crate error and we read it together.\n${room.shareUrl}`,
    );
    const x = new URL(
      screen.getByRole('link', { name: 'Post to X (opens in a new tab)' }).getAttribute('href') ??
        '',
    );
    expect(x.searchParams.get('url')).toBe(room.shareUrl);
    expect(
      screen.getByRole('link', { name: 'Post to Weibo (opens in a new tab)' }),
    ).toHaveAttribute('target', '_blank');
    expect(
      screen.getByRole('img', { name: 'QR code for the link to Rust help desk' }),
    ).toBeVisible();
    expect(screen.getByText('Rust help desk · Agent Room')).toBeVisible();
    expect(screen.getByText('agentroom.test')).toBeVisible();
  });

  it('改了的话跟着带到分享网址上', () => {
    renderDialog();
    fireEvent.change(screen.getByRole('textbox', { name: 'What you’ll say' }), {
      target: { value: `Rust people, come by! ${room.shareUrl}` },
    });
    const reddit = new URL(
      screen
        .getByRole('link', { name: 'Post to Reddit (opens in a new tab)' })
        .getAttribute('href') ?? '',
    );
    expect(reddit.searchParams.get('title')).toBe('Rust people, come by!');
  });

  it('中文：默认的话用「」，微博写“微博”', async () => {
    await i18n.changeLanguage('zh-CN');
    renderDialog();
    expect(screen.getByRole('textbox', { name: '想说的话' })).toHaveValue(
      `来「Rust help desk」一起聊聊吧：Bring a crate error and we read it together.
${room.shareUrl}`,
    );
    expect(screen.getByRole('link', { name: '发到微博（在新标签页打开）' })).toBeVisible();
  });

  it('有系统的分享菜单才给“更多…”', () => {
    renderDialog();
    expect(screen.queryByRole('button', { name: 'More…' })).toBeNull();
    cleanup();

    const share = vi.fn(() => Promise.resolve());
    Object.defineProperty(window.navigator, 'share', { configurable: true, value: share });
    renderDialog();
    fireEvent.click(screen.getByRole('button', { name: 'More…' }));
    expect(share).toHaveBeenCalledWith({
      text: 'Come chat in “Rust help desk”: Bring a crate error and we read it together.',
      title: 'Rust help desk · Agent Room',
      url: room.shareUrl,
    });
  });
});
