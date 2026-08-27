// @vitest-environment jsdom

import '@testing-library/jest-dom/vitest';

import { cleanup, render, screen } from '@testing-library/react';
import userEvent from '@testing-library/user-event';
import { I18nextProvider } from 'react-i18next';
import { afterEach, beforeAll, describe, expect, it, vi } from 'vitest';

import { useAppServices } from '@/app/app-services';
import { LandingPage } from '@/features/landing/ui/landing-page';
import { i18n, initializeI18n } from '@/shared/i18n/i18n';

vi.mock('@/app/app-services', () => ({ useAppServices: vi.fn() }));
vi.mock('motion/react', () => ({ motion: { aside: 'aside', div: 'div' } }));

beforeAll(async () => {
  await initializeI18n(window.localStorage, ['en']);
});

afterEach(() => {
  cleanup();
  vi.clearAllMocks();
});

describe('公开 Alpha 首页', () => {
  it('只有配置版本化资产时才提供下载链接', () => {
    configure('https://download.agent-room.test/v0.1.0-alpha.1/installer.exe');

    renderPage();

    expect(screen.getByRole('link', { name: 'Download Windows client' })).toHaveAttribute(
      'href',
      'https://download.agent-room.test/v0.1.0-alpha.1/installer.exe',
    );
  });

  it('资产尚未发布时显示不可点击的可诊断状态', () => {
    configure(null);

    renderPage();

    expect(screen.getByRole('button', { name: 'Windows Alpha coming soon' })).toBeDisabled();
    expect(screen.queryByRole('link', { name: 'Download Windows client' })).not.toBeInTheDocument();
  });

  it('注册入口发送明确的注册意图', async () => {
    const user = userEvent.setup();
    const beginAuthentication = vi.fn();
    vi.mocked(useAppServices).mockReturnValue({
      config: { registrationMode: 'open-email', windowsDownloadUrl: null },
      controlPlane: { beginAuthentication },
    } as unknown as ReturnType<typeof useAppServices>);

    renderPage();
    await user.click(screen.getByRole('button', { name: 'Create account' }));

    expect(beginAuthentication).toHaveBeenCalledWith('/connect', 'register');
  });

  it('服务端关闭注册时不给用户死入口', () => {
    configure(null);

    renderPage();

    expect(screen.getByRole('button', { name: 'Registration coming soon' })).toBeDisabled();
    expect(screen.queryByRole('button', { name: 'Create account' })).not.toBeInTheDocument();
  });
});

function configure(windowsDownloadUrl: string | null) {
  vi.mocked(useAppServices).mockReturnValue({
    config: { registrationMode: 'closed', windowsDownloadUrl },
    controlPlane: { beginAuthentication: vi.fn() },
  } as unknown as ReturnType<typeof useAppServices>);
}

function renderPage() {
  return render(
    <I18nextProvider i18n={i18n}>
      <LandingPage />
    </I18nextProvider>,
  );
}
