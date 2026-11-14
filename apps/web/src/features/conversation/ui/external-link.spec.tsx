// @vitest-environment jsdom
import '@testing-library/jest-dom/vitest';
import { cleanup, fireEvent, render, screen, waitFor, within } from '@testing-library/react';
import { I18nextProvider } from 'react-i18next';
import { afterEach, beforeAll, beforeEach, describe, expect, it, vi } from 'vitest';
import { i18n, initializeI18n } from '@/shared/i18n/i18n';
import { ChatMarkdown } from './chat-markdown';
import { ExternalLinkGate } from './external-link';

const desktop = vi.hoisted(() => ({
  controller: null as {
    readonly available: boolean;
    readonly openLink?: (url: string) => Promise<boolean>;
  } | null,
}));
vi.mock('@/features/desktop/ui/desktop-runtime-provider', () => ({
  useOptionalDesktopRuntimeController: () => desktop.controller,
}));

const address = 'https://docs.example.com/guide/start?from=room#install';
const windowOpen = vi.fn<typeof window.open>(() => null);

beforeAll(async () => {
  await initializeI18n(window.localStorage, ['en']);
});
beforeEach(() => {
  desktop.controller = null;
  windowOpen.mockClear();
  vi.spyOn(window, 'open').mockImplementation(windowOpen);
});
afterEach(() => {
  cleanup();
  vi.restoreAllMocks();
});

function renderMessage(source = `Read ${address} first.`) {
  return render(
    <I18nextProvider i18n={i18n}>
      <ExternalLinkGate>
        <ChatMarkdown source={source} />
      </ExternalLinkGate>
    </I18nextProvider>,
  );
}

const link = () => screen.getByRole('link', { name: address });
const dialog = () => screen.getByRole('dialog', { name: 'Open this link?' });

describe('正文里的地址', () => {
  it('没有确认框接着的地方只是字，不占 Tab', () => {
    const view = render(
      <I18nextProvider i18n={i18n}>
        <ChatMarkdown source={`Read ${address} first.`} />
      </I18nextProvider>,
    );

    expect(screen.queryByRole('link')).toBeNull();
    expect(view.container.querySelector('[tabindex]')).toBeNull();
    expect(view.container.textContent).toBe(`Read ${address} first.`);
  });

  it('能点、能用键盘走到，但不是带地址的链接：中键、右键菜单绕不过确认', () => {
    const view = renderMessage();

    expect(link()).toHaveAttribute('tabindex', '0');
    expect(link()).not.toHaveAttribute('href');
    expect(view.container.querySelector('a')).toBeNull();
    expect(view.container.textContent).toBe(`Read ${address} first.`);
    expect(screen.queryByRole('dialog')).toBeNull();
  });

  it('点了先问：域名单独一行，完整的地址原样写着；取消就什么也不打开', () => {
    renderMessage();

    fireEvent.click(link());

    expect(within(dialog()).getByText('docs.example.com')).toHaveClass('external-link__site');
    expect(within(dialog()).getByText(address)).toHaveClass('external-link__address');
    expect(within(dialog()).getByText('It opens in a new tab.')).toBeVisible();

    fireEvent.click(within(dialog()).getByRole('button', { name: 'Cancel' }));

    expect(screen.queryByRole('dialog')).toBeNull();
    expect(windowOpen).not.toHaveBeenCalled();
  });

  it('域名带端口时端口也写出来', () => {
    renderMessage('Dev server: http://localhost:5173/app');

    fireEvent.click(screen.getByRole('link'));

    expect(within(dialog()).getByText('localhost:5173')).toHaveClass('external-link__site');
  });

  it('确认了才打开：开在新标签页里，不带来源', () => {
    renderMessage();

    fireEvent.click(link());
    fireEvent.click(within(dialog()).getByRole('button', { name: 'Open' }));

    expect(windowOpen).toHaveBeenCalledExactlyOnceWith(address, '_blank', 'noopener,noreferrer');
    expect(screen.queryByRole('dialog')).toBeNull();
  });

  it('键盘上按回车也是先问；别的键不管', () => {
    renderMessage();

    fireEvent.keyDown(link(), { key: ' ' });
    expect(screen.queryByRole('dialog')).toBeNull();

    fireEvent.keyDown(link(), { key: 'Enter' });
    expect(dialog()).toBeVisible();
  });

  it('确认框里能复制地址', async () => {
    const writeText = vi.fn(() => Promise.resolve());
    Object.defineProperty(navigator, 'clipboard', { configurable: true, value: { writeText } });
    renderMessage();

    fireEvent.click(link());
    fireEvent.click(within(dialog()).getByRole('button', { name: 'Copy address' }));

    await waitFor(() => {
      expect(within(dialog()).getByRole('button', { name: 'Copied' })).toBeVisible();
    });
    expect(writeText).toHaveBeenCalledExactlyOnceWith(address);
    expect(windowOpen).not.toHaveBeenCalled();
  });
});

describe('桌面端打开正文里的地址', () => {
  it('交给原生层在系统浏览器里打开，窗口自己不开', async () => {
    const openLink = vi.fn(() => Promise.resolve(true));
    desktop.controller = { available: true, openLink };
    renderMessage();

    fireEvent.click(link());
    expect(within(dialog()).getByText('It opens in your browser.')).toBeVisible();
    fireEvent.click(within(dialog()).getByRole('button', { name: 'Open' }));

    await waitFor(() => {
      expect(screen.queryByRole('dialog')).toBeNull();
    });
    expect(openLink).toHaveBeenCalledExactlyOnceWith(address);
    expect(windowOpen).not.toHaveBeenCalled();
  });

  it('没打开就留着确认框，说一声，地址还能复制', async () => {
    desktop.controller = { available: true, openLink: () => Promise.resolve(false) };
    renderMessage();

    fireEvent.click(link());
    fireEvent.click(within(dialog()).getByRole('button', { name: 'Open' }));

    expect(await within(dialog()).findByRole('alert')).toHaveTextContent(
      'Could not open it. Copy the address and open it in your browser.',
    );
    expect(within(dialog()).getByRole('button', { name: 'Copy address' })).toBeEnabled();
    expect(within(dialog()).getByRole('button', { name: 'Open' })).toBeEnabled();
    expect(windowOpen).not.toHaveBeenCalled();
  });

  it('网页端（没有桌面运行时）照旧开新标签页', () => {
    desktop.controller = { available: false };
    renderMessage();

    fireEvent.click(link());
    fireEvent.click(within(dialog()).getByRole('button', { name: 'Open' }));

    expect(windowOpen).toHaveBeenCalledExactlyOnceWith(address, '_blank', 'noopener,noreferrer');
  });
});
