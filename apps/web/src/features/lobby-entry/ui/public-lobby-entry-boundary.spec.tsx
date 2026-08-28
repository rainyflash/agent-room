// @vitest-environment jsdom

import '@testing-library/jest-dom/vitest';

import { QueryClient, QueryClientProvider } from '@tanstack/react-query';
import { cleanup, render, screen } from '@testing-library/react';
import { I18nextProvider } from 'react-i18next';
import { afterEach, beforeAll, describe, expect, it, vi } from 'vitest';

import { useAppServices } from '@/app/app-services';
import type { PublicLobbyEntryTarget } from '@/features/lobby-entry/domain/public-lobby-entry';
import { PublicLobbyEntryBoundary } from '@/features/lobby-entry/ui/public-lobby-entry-boundary';
import { useSession } from '@/features/session/ui/session-provider';
import { i18n, initializeI18n } from '@/shared/i18n/i18n';
import { err } from '@/shared/result';

vi.mock('@/app/app-services', () => ({ useAppServices: vi.fn() }));
vi.mock('@/features/preferences/ui/language-control', () => ({ LanguageControl: () => null }));
vi.mock('@/features/session/ui/session-provider', () => ({ useSession: vi.fn() }));

const catalogId = '0198b601-77a2-7f41-b4f4-940f291951b8';

beforeAll(async () => {
  await initializeI18n(window.localStorage, ['en']);
});

afterEach(() => {
  cleanup();
  vi.clearAllMocks();
});

describe('公开大厅入场边界', () => {
  it('没有活跃房间时呈现可恢复等待态而不是故障', async () => {
    const enter = vi.fn(async () => err({ code: 'lobby.observation_not_found', retryable: false }));
    const onEntered = vi.fn();
    vi.mocked(useAppServices).mockReturnValue({
      lobbyEntry: { enter },
    } as unknown as ReturnType<typeof useAppServices>);
    vi.mocked(useSession).mockReturnValue({
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

    renderBoundary(onEntered);

    expect(
      await screen.findByRole('heading', { name: 'Waiting for the first Agent.' }),
    ).toBeVisible();
    expect(screen.getByText(/no Runtime has opened its first live room yet/u)).toBeVisible();
    expect(screen.getByRole('button', { name: 'Check now' })).toBeEnabled();
    expect(screen.queryByText('lobby.observation_not_found')).not.toBeInTheDocument();
    expect(enter).toHaveBeenCalledWith(catalogId);
    expect(onEntered).not.toHaveBeenCalled();
  });
});

function renderBoundary(onEntered: (target: PublicLobbyEntryTarget) => void) {
  const queryClient = new QueryClient({
    defaultOptions: { queries: { retry: false } },
  });
  return render(
    <I18nextProvider i18n={i18n}>
      <QueryClientProvider client={queryClient}>
        <PublicLobbyEntryBoundary
          catalogId={catalogId}
          onConnectionRequired={vi.fn()}
          onEntered={onEntered}
        />
      </QueryClientProvider>
    </I18nextProvider>,
  );
}
