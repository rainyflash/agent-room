// @vitest-environment jsdom
import '@testing-library/jest-dom/vitest';
import { cleanup, fireEvent, render, screen, waitFor } from '@testing-library/react';
import { I18nextProvider } from 'react-i18next';
import { afterEach, beforeAll, beforeEach, describe, expect, it, vi } from 'vitest';

import { useOptionalDesktopRuntimeController } from '@/features/desktop/ui/desktop-runtime-provider';
import type { ShareCardContent } from '@/features/room-share/domain/share-card';
import { saveBlob } from '@/shared/browser/save-file';
import { i18n, initializeI18n } from '@/shared/i18n/i18n';

import { ShareCard } from './share-card';
import { renderShareCard } from './share-card-canvas';

// jsdom 没有画布：画的那一段在浏览器验收里查，这里只查拿到图以后怎么摆、怎么发。
vi.mock('./share-card-canvas', () => ({ renderShareCard: vi.fn() }));
vi.mock('@/shared/browser/save-file', () => ({ saveBlob: vi.fn() }));
vi.mock('@/features/desktop/ui/desktop-runtime-provider', () => ({
  useOptionalDesktopRuntimeController: vi.fn(() => null),
}));

const content: ShareCardContent = {
  action: 'Scan to join',
  lead: 'Mina invites you to a room',
  notes: ['Sign in and you’re in', 'Valid until Oct 21'],
  roomName: 'Weekly review',
  tagline: 'Rooms where people and AI agents talk',
  tone: 'teal',
  url: 'https://agentroom.test/join#secret',
};
const image = new Blob(['png'], { type: 'image/png' });

class FakeClipboardItem {
  constructor(readonly items: Record<string, Blob>) {}
}

function renderCard(card: ShareCardContent = content) {
  return render(
    <I18nextProvider i18n={i18n}>
      <ShareCard
        alt="Invite card for Weekly review"
        content={card}
        detail="The QR code is this link."
        heading="Invite card"
      />
    </I18nextProvider>,
  );
}

function stubClipboard(write: (items: readonly FakeClipboardItem[]) => Promise<void>) {
  vi.stubGlobal('ClipboardItem', FakeClipboardItem);
  Object.defineProperty(navigator, 'clipboard', { configurable: true, value: { write } });
}

beforeAll(async () => {
  await initializeI18n(window.localStorage, ['en']);
});

const revokeObjectUrl = vi.fn<(url: string) => void>();

beforeEach(() => {
  vi.mocked(renderShareCard).mockResolvedValue(image);
  URL.createObjectURL = vi.fn(() => 'blob:card');
  URL.revokeObjectURL = revokeObjectUrl;
});

afterEach(() => {
  cleanup();
  vi.unstubAllGlobals();
  vi.clearAllMocks();
  vi.mocked(useOptionalDesktopRuntimeController).mockReturnValue(null);
  Reflect.deleteProperty(navigator, 'clipboard');
  Reflect.deleteProperty(navigator, 'canShare');
  Reflect.deleteProperty(navigator, 'share');
});

describe('发给人的那张卡片', () => {
  it('画好以前按钮先灰着；画好了是一张图，关掉时收回做好的地址', async () => {
    const view = renderCard();

    expect(screen.getByRole('button', { name: 'Save image' })).toBeDisabled();
    const card = await screen.findByRole('img', { name: 'Invite card for Weekly review' });
    expect(card).toHaveAttribute('src', 'blob:card');
    expect(renderShareCard).toHaveBeenCalledWith(content);
    expect(screen.getByRole('button', { name: 'Save image' })).toBeEnabled();
    expect(screen.getByText('The QR code is this link.')).toBeVisible();

    view.unmount();
    expect(revokeObjectUrl).toHaveBeenCalledWith('blob:card');
  });

  it('复制图片：把这张图放进剪贴板，按钮换成“已复制”', async () => {
    const write = vi.fn<(items: readonly FakeClipboardItem[]) => Promise<void>>(() =>
      Promise.resolve(),
    );
    stubClipboard(write);
    renderCard();
    await screen.findByRole('img', { name: 'Invite card for Weekly review' });

    fireEvent.click(screen.getByRole('button', { name: 'Copy image' }));

    expect(await screen.findByRole('button', { name: 'Image copied' })).toBeVisible();
    expect(write.mock.calls[0]?.[0][0]?.items).toEqual({ 'image/png': image });
  });

  it('没复制上：说一句，叫人存下来再发', async () => {
    stubClipboard(() => Promise.reject(new Error('denied')));
    renderCard();
    await screen.findByRole('img', { name: 'Invite card for Weekly review' });

    fireEvent.click(screen.getByRole('button', { name: 'Copy image' }));

    expect(await screen.findByRole('status')).toHaveTextContent(
      'Couldn’t copy the image. Save it and send the file instead.',
    );
  });

  it('这个浏览器放不了图进剪贴板时不给“复制图片”，照样能存', async () => {
    renderCard();
    await screen.findByRole('img', { name: 'Invite card for Weekly review' });

    expect(screen.queryByRole('button', { name: 'Copy image' })).toBeNull();
    fireEvent.click(screen.getByRole('button', { name: 'Save image' }));

    expect(saveBlob).toHaveBeenCalledWith(image, 'agent-room-Weekly review.png');
  });

  it('桌面端存图片：原生层存，按钮下面说存到哪了、能在文件夹里指出来；换了图就不说了', async () => {
    const saveFile = vi.fn<(fileName: string, content: Blob) => Promise<string | null>>(() =>
      Promise.resolve('agent-room-Weekly review (1).png'),
    );
    const showSavedFile = vi.fn<(fileName: string) => Promise<boolean>>(() =>
      Promise.resolve(true),
    );
    vi.mocked(useOptionalDesktopRuntimeController).mockReturnValue({
      available: true,
      saveFile,
      showSavedFile,
    } as unknown as ReturnType<typeof useOptionalDesktopRuntimeController>);
    const view = renderCard();
    await screen.findByRole('img', { name: 'Invite card for Weekly review' });

    fireEvent.click(screen.getByRole('button', { name: 'Save image' }));

    expect(await screen.findByRole('status')).toHaveTextContent(
      'Saved to your Downloads folder as agent-room-Weekly review (1).png',
    );
    expect(saveFile).toHaveBeenCalledWith('agent-room-Weekly review.png', image);
    expect(saveBlob).not.toHaveBeenCalled();
    fireEvent.click(screen.getByRole('button', { name: 'Show in folder' }));
    expect(showSavedFile).toHaveBeenCalledWith('agent-room-Weekly review (1).png');

    // 换了一条链接，画的是另一张图：上一张存到哪了不再挂着。
    vi.mocked(renderShareCard).mockResolvedValue(new Blob(['next'], { type: 'image/png' }));
    view.rerender(
      <I18nextProvider i18n={i18n}>
        <ShareCard
          alt="Invite card for Weekly review"
          content={{ ...content, url: 'https://agentroom.test/join#next' }}
          detail="The QR code is this link."
          heading="Invite card"
        />
      </I18nextProvider>,
    );
    await waitFor(() => {
      expect(screen.queryByRole('status')).toBeNull();
    });
    expect(screen.queryByRole('button', { name: 'Show in folder' })).toBeNull();
  });

  it('系统的分享菜单收图片时才给“发给…”，交过去的是这张图', async () => {
    const share = vi.fn<(data: ShareData) => Promise<void>>(() => Promise.resolve());
    Object.defineProperty(navigator, 'canShare', {
      configurable: true,
      value: (data: ShareData) => (data.files?.length ?? 0) > 0,
    });
    Object.defineProperty(navigator, 'share', { configurable: true, value: share });
    renderCard();
    await screen.findByRole('img', { name: 'Invite card for Weekly review' });

    fireEvent.click(screen.getByRole('button', { name: 'Send to…' }));

    const files = share.mock.calls[0]?.[0].files ?? [];
    expect(files.map((file) => [file.name, file.type])).toEqual([
      ['agent-room-Weekly review.png', 'image/png'],
    ]);
  });

  it('系统的分享菜单不收图片时没有“发给…”', async () => {
    Object.defineProperty(navigator, 'canShare', { configurable: true, value: () => false });
    renderCard();
    await screen.findByRole('img', { name: 'Invite card for Weekly review' });

    expect(screen.queryByRole('button', { name: 'Send to…' })).toBeNull();
  });

  it('内容换了重画一张，上一张的地址收回', async () => {
    const view = renderCard();
    await screen.findByRole('img', { name: 'Invite card for Weekly review' });
    const next = { ...content, url: 'https://agentroom.test/join#next' };

    view.rerender(
      <I18nextProvider i18n={i18n}>
        <ShareCard
          alt="Invite card for Weekly review"
          content={next}
          detail="The QR code is this link."
          heading="Invite card"
        />
      </I18nextProvider>,
    );

    await waitFor(() => {
      expect(renderShareCard).toHaveBeenLastCalledWith(next);
    });
    expect(revokeObjectUrl).toHaveBeenCalledWith('blob:card');
    expect(await screen.findByRole('img', { name: 'Invite card for Weekly review' })).toBeVisible();
  });

  it('画不出来时这一块不出现', async () => {
    vi.mocked(renderShareCard).mockRejectedValue(new Error('no canvas'));
    renderCard();

    await waitFor(() => {
      expect(screen.queryByRole('heading', { name: 'Invite card' })).toBeNull();
    });
  });
});
