// @vitest-environment jsdom

import '@testing-library/jest-dom/vitest';

import { act, cleanup, fireEvent, render, screen, waitFor } from '@testing-library/react';
import { I18nextProvider } from 'react-i18next';
import { afterEach, beforeAll, beforeEach, describe, expect, it, vi } from 'vitest';

import { AppServicesProvider, type AppServices } from '@/app/app-services';
import type { SharePreviewGateway, ShareableRoom } from '@/features/room-share/domain/room-share';
import { renderShareCard } from '@/features/room-share/ui/share-card-canvas';
import { ShareRoomDialog } from '@/features/room-share/ui/share-room-dialog';
import { i18n, initializeI18n } from '@/shared/i18n/i18n';
import { roomTone } from '@/shared/ui/room-tone';

// jsdom 没有画布：卡片画成什么样在浏览器验收里查，这里查交给它画的是什么。挂着不回来，
// 免得它去做 `blob:` 地址，和下面数预览图地址的用例搅在一起。
vi.mock('@/features/room-share/ui/share-card-canvas', () => ({
  renderShareCard: vi.fn(() => new Promise<Blob>(() => undefined)),
}));

const room: ShareableRoom = {
  catalogId: '0198c000-0000-7000-8000-000000000001',
  description: 'Bring a crate error and we read it together.',
  name: 'Rust help desk',
  shareCode: 'aB3xYz',
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
  Reflect.deleteProperty(URL, 'createObjectURL');
  Reflect.deleteProperty(URL, 'revokeObjectURL');
});

function dialog(previews: SharePreviewGateway | undefined, shown: ShareableRoom) {
  const element = (
    <I18nextProvider i18n={i18n}>
      <ShareRoomDialog onClose={() => undefined} room={shown} />
    </I18nextProvider>
  );
  return previews === undefined ? (
    element
  ) : (
    <AppServicesProvider services={{ sharePreviews: previews } as unknown as AppServices}>
      {element}
    </AppServicesProvider>
  );
}

function renderDialog(previews?: SharePreviewGateway, shown: ShareableRoom = room) {
  return render(dialog(previews, shown));
}

/** jsdom 没有 `createObjectURL`：换成假的，每次给一个新地址，记下收回了哪些。 */
function objectUrls() {
  let next = 0;
  const create = vi.fn<(image: Blob) => string>(() => {
    next += 1;
    return `blob:preview-${String(next)}`;
  });
  const revoke = vi.fn<(url: string) => void>();
  Object.defineProperty(URL, 'createObjectURL', { configurable: true, value: create });
  Object.defineProperty(URL, 'revokeObjectURL', { configurable: true, value: revoke });
  return { create, revoke };
}

type PendingCall = {
  readonly shareCode: string;
  readonly signal: AbortSignal;
  readonly answer: (image: Blob | null) => void;
};

/** 一个先挂着、由用例决定什么时候回来的预览图来源。 */
function pendingPreviews() {
  const calls: PendingCall[] = [];
  const gateway: SharePreviewGateway = {
    roomImage: (shareCode, signal) =>
      new Promise((resolve) => {
        calls.push({ shareCode, signal, answer: resolve });
      }),
  };
  return { calls, gateway };
}

async function answer(call: PendingCall | undefined, image: Blob | null) {
  await act(async () => {
    call?.answer(image);
    await Promise.resolve();
  });
}

const png = new Blob([new Uint8Array([0x89, 0x50, 0x4e, 0x47])], { type: 'image/png' });

function previewImage(): HTMLImageElement | null {
  return document.querySelector('.room-share__preview img');
}

function previewFigure(): HTMLElement | null {
  return document.querySelector('.room-share__preview figure');
}

describe('分享对话框', () => {
  it('链接、默认的一段话、三家的分享网址、分享卡片和预览', () => {
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
    // 分享卡片：公开房间不写是谁分享的，二维码是这间房的分享链接。
    expect(screen.getByRole('heading', { name: 'Share card' })).toBeVisible();
    expect(renderShareCard).toHaveBeenCalledWith({
      action: 'Scan to join',
      lead: 'Public room',
      notes: ['No app to install', 'Sign in to join or ask to join'],
      roomName: 'Rust help desk',
      tagline: 'Rooms where people and AI agents talk',
      tone: roomTone(room.catalogId),
      url: room.shareUrl,
    });
    expect(screen.getByText('Rust help desk · Agent Room')).toBeVisible();
    expect(screen.getByText('agentroom.test')).toBeVisible();
    expect(document.querySelector('.room-share__preview img')).toHaveAttribute(
      'src',
      '/room-share-preview-en.png',
    );
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

  it('发出去长这样：放这间房自己的图，关掉对话框时收回地址', async () => {
    const urls = objectUrls();
    const previews = pendingPreviews();
    const view = renderDialog(previews.gateway);
    // 要着的那一会儿空着一块，不先放通用图。
    expect(previewImage()).toBeNull();
    expect(previewFigure()).toHaveAttribute('aria-busy', 'true');
    expect(previews.calls.map(({ shareCode }) => shareCode)).toEqual(['aB3xYz']);

    await answer(previews.calls[0], png);
    expect(urls.create).toHaveBeenCalledWith(png);
    expect(previewImage()).toHaveAttribute('src', 'blob:preview-1');
    expect(previewImage()).toHaveAttribute('data-preview', 'room');
    expect(previewFigure()).not.toHaveAttribute('aria-busy');
    expect(screen.getByText('Rust help desk · Agent Room')).toBeVisible();

    view.unmount();
    expect(urls.revoke).toHaveBeenCalledWith('blob:preview-1');
    expect(previews.calls[0]?.signal.aborted).toBe(true);
  });

  it('要不到这间房的图就放通用图：没有、出错、图坏了', async () => {
    const urls = objectUrls();
    renderDialog({ roomImage: () => Promise.resolve(null) });
    await waitFor(() => {
      expect(previewImage()).toHaveAttribute('src', '/room-share-preview-en.png');
    });
    expect(previewImage()).toHaveAttribute('data-preview', 'generic');
    cleanup();

    renderDialog({ roomImage: () => Promise.reject(new TypeError('offline')) });
    await waitFor(() => {
      expect(previewImage()).toHaveAttribute('src', '/room-share-preview-en.png');
    });
    cleanup();

    // 拿到了字节，浏览器却画不出来：换回通用图，地址照样在关掉时收回。
    const view = renderDialog({ roomImage: () => Promise.resolve(png) });
    await waitFor(() => {
      expect(previewImage()).toHaveAttribute('src', 'blob:preview-1');
    });
    const broken = previewImage();
    if (broken === null) throw new Error('预览图不见了');
    fireEvent.error(broken);
    expect(previewImage()).toHaveAttribute('src', '/room-share-preview-en.png');
    view.unmount();
    expect(urls.revoke).toHaveBeenCalledWith('blob:preview-1');
  });

  it('不知道分享码时不去要，中文名字放中文的通用图', () => {
    const roomImage = vi.fn<SharePreviewGateway['roomImage']>();
    renderDialog({ roomImage }, { ...room, name: 'Rust 读书会', shareCode: null });
    expect(roomImage).not.toHaveBeenCalled();
    expect(previewImage()).toHaveAttribute('src', '/room-share-preview-zh-CN.png');
  });

  it('图还没回来就关掉：停下请求，回来了也不做地址', async () => {
    const urls = objectUrls();
    const previews = pendingPreviews();
    const view = renderDialog(previews.gateway);
    view.unmount();
    expect(previews.calls[0]?.signal.aborted).toBe(true);
    await answer(previews.calls[0], png);
    expect(urls.create).not.toHaveBeenCalled();
    expect(urls.revoke).not.toHaveBeenCalled();
  });

  it('改了房间名重新要一张，上一张的地址收回', async () => {
    const urls = objectUrls();
    const previews = pendingPreviews();
    const view = renderDialog(previews.gateway);
    await answer(previews.calls[0], png);
    expect(previewImage()).toHaveAttribute('src', 'blob:preview-1');

    view.rerender(dialog(previews.gateway, { ...room, name: 'Rust office hours' }));
    expect(urls.revoke).toHaveBeenCalledWith('blob:preview-1');
    expect(previews.calls).toHaveLength(2);
    // 收回的那个地址不再放。
    expect(previewImage()).toBeNull();
    await answer(previews.calls[1], png);
    expect(previewImage()).toHaveAttribute('src', 'blob:preview-2');
  });
});
