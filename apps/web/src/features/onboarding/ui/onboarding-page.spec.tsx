// @vitest-environment jsdom

import '@testing-library/jest-dom/vitest';

import { QueryClient, QueryClientProvider } from '@tanstack/react-query';
import { cleanup, render, screen } from '@testing-library/react';
import { I18nextProvider } from 'react-i18next';
import { afterEach, beforeAll, beforeEach, describe, expect, it, vi } from 'vitest';

import { useAppServices } from '@/app/app-services';
import { useDesktopRuntime } from '@/features/desktop/ui/use-desktop-runtime';
import { OnboardingPage } from '@/features/onboarding/ui/onboarding-page';
import { useSession } from '@/features/session/ui/session-provider';
import { i18n, initializeI18n } from '@/shared/i18n/i18n';
import { ok } from '@/shared/result';

vi.mock('@/app/app-services', () => ({ useAppServices: vi.fn() }));
vi.mock('@/features/desktop/ui/use-desktop-runtime', () => ({
  useDesktopRuntime: vi.fn(),
}));
vi.mock('@/features/session/ui/session-provider', () => ({ useSession: vi.fn() }));
vi.mock('motion/react', () => ({
  motion: { article: 'article', header: 'header' },
  useReducedMotion: () => true,
}));

const agent = {
  agentId: '0198b601-77a1-7bb8-83eb-a8fe68c97e44',
  avatarContentId: null,
  description: '',
  displayName: 'Build Agent',
  matrixUserId: '@agent:matrix.test',
  registeredAtUnixMs: 1,
  slug: 'build-agent',
  visibility: 'private' as const,
};

const lobby = {
  activeInstanceCount: 2,
  catalogId: '0198b601-77a2-7f41-b4f4-940f291951b8',
  description: 'English public lobby',
  language: 'en',
  name: 'English lobby',
  onlineAgentCount: 7,
  slug: 'english',
};

beforeAll(async () => {
  await initializeI18n(window.localStorage, ['en']);
});

beforeEach(() => {
  window.localStorage.clear();
  vi.mocked(useSession).mockReturnValue({
    send: vi.fn(),
    snapshot: {
      context: {
        principal: {
          displayName: 'Alice',
          locale: 'en',
          matrixUserId: '@alice:matrix.test',
          principalId: '0198b601-77a3-74f1-b4f4-940f291951b9',
        },
      },
      value: 'ready',
    },
  } as unknown as ReturnType<typeof useSession>);
  vi.mocked(useDesktopRuntime).mockReturnValue({
    available: false,
    busy: null,
    checkUpdate: vi.fn(),
    configureAgentRuntime: vi.fn(),
    configureHost: vi.fn(),
    dismissFailure: vi.fn(),
    failure: null,
    hosts: [],
    installUpdate: vi.fn(),
    openAuthorization: vi.fn(),
    refresh: vi.fn(),
    retryBridge: vi.fn(),
    setAutostart: vi.fn(),
    snapshot: null,
    update: null,
  });
});

afterEach(() => {
  cleanup();
  vi.clearAllMocks();
});

describe('首次引导页面', () => {
  it('从服务端事实展示账户、首个 Agent、真实大厅与 Windows Runtime 入口', async () => {
    const bootstrap = vi.fn(async () => ok({ agent, lobby, reusedExistingAgent: false as const }));
    vi.mocked(useAppServices).mockReturnValue({
      config: {
        windowsDownloadUrl: 'https://download.agent-room.test/windows',
      },
      desktop: {},
      onboarding: { bootstrap },
    } as unknown as ReturnType<typeof useAppServices>);

    renderPage();

    expect(
      await screen.findByRole('heading', { name: 'Bring your first Agent online.' }),
    ).toBeVisible();
    expect(await screen.findByText('Build Agent')).toBeVisible();
    expect(screen.getAllByText('Alice')).toHaveLength(2);
    expect(screen.getByText('English lobby')).toBeVisible();
    expect(screen.getByText('7 agents online · 2 active rooms')).toBeVisible();
    expect(screen.getByRole('link', { name: 'Download Windows client' })).toHaveAttribute(
      'href',
      'https://download.agent-room.test/windows',
    );
    expect(bootstrap).toHaveBeenCalledWith('en');
  });
});

function renderPage() {
  const queryClient = new QueryClient({
    defaultOptions: { mutations: { retry: false }, queries: { retry: false } },
  });
  return render(
    <I18nextProvider i18n={i18n}>
      <QueryClientProvider client={queryClient}>
        <OnboardingPage />
      </QueryClientProvider>
    </I18nextProvider>,
  );
}
