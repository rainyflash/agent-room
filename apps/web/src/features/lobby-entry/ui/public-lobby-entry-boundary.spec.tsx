// @vitest-environment jsdom

import '@testing-library/jest-dom/vitest';

import { QueryClient, QueryClientProvider } from '@tanstack/react-query';
import { cleanup, render, screen } from '@testing-library/react';
import { I18nextProvider } from 'react-i18next';
import { afterEach, beforeAll, describe, expect, it, vi } from 'vitest';

import { useAppServices } from '@/app/app-services';
import type { PublicLobbyRouteTarget } from '@/features/lobby-entry/domain/public-lobby-entry';
import { PublicLobbyEntryBoundary } from '@/features/lobby-entry/ui/public-lobby-entry-boundary';
import type { PrivateRoom } from '@/features/private-rooms/domain/private-room';
import { useSession } from '@/features/session/ui/session-provider';
import { i18n, initializeI18n } from '@/shared/i18n/i18n';
import { err, ok } from '@/shared/result';

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
  it('云端并发供给时呈现可恢复准备态而不是依赖本机 Runtime', async () => {
    const enter = vi.fn(() =>
      Promise.resolve(err({ code: 'lobby.entry_provisioning_busy', retryable: true })),
    );
    const onEntered = vi.fn();
    mockServices(enter);
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

    expect(await screen.findByRole('heading', { name: 'Setting up the room' })).toBeVisible();
    expect(screen.getByText(/Nothing to install/u)).toBeVisible();
    expect(screen.getByRole('button', { name: 'Check now' })).toBeEnabled();
    expect(screen.queryByText('lobby.entry_provisioning_busy')).not.toBeInTheDocument();
    expect(enter).toHaveBeenCalledWith(catalogId);
    expect(onEntered).not.toHaveBeenCalled();
  });

  it('进不去时说人话、给“回到房间”，错误码收进详情', async () => {
    const enter = vi.fn(() =>
      Promise.resolve(err({ code: 'lobby.entry_catalog_missing', retryable: false })),
    );
    mockServices(enter);
    mockSession('ready', principal);

    renderBoundary(vi.fn());

    expect(await screen.findByRole('heading', { name: 'Could not enter the room' })).toBeVisible();
    expect(screen.getByRole('button', { name: 'Try again' })).toBeEnabled();
    expect(screen.getByRole('link', { name: 'Back to rooms' })).toHaveAttribute('href', '/rooms');
    expect(screen.getByText('lobby.entry_catalog_missing')).not.toBeVisible();
    expect(screen.getByText('Details')).toBeVisible();
  });

  it('没登录时交给路由去登录，不在这里请求进房间', () => {
    const enter = vi.fn();
    const list = mockServices(enter);
    mockSession('unauthenticated', null);
    const onConnectionRequired = vi.fn();

    renderBoundary(vi.fn(), onConnectionRequired);

    expect(onConnectionRequired).toHaveBeenCalledOnce();
    expect(enter).not.toHaveBeenCalled();
    expect(list).not.toHaveBeenCalled();
  });

  it('你在里面的私人房间直接打开，不走公开大厅入场', async () => {
    const enter = vi.fn();
    mockServices(enter, [privateRoom('joined')]);
    mockSession('ready', principal);
    const onEntered = vi.fn();

    renderBoundary(onEntered);

    await vi.waitFor(() => {
      expect(onEntered).toHaveBeenCalledWith({ catalogId, matrixRoomId: privateMatrixRoomId });
    });
    expect(onEntered).toHaveBeenCalledOnce();
    expect(enter).not.toHaveBeenCalled();
  });

  it('还没答复的邀请请人去房间列表点加入，不替他加入', async () => {
    const enter = vi.fn();
    mockServices(enter, [privateRoom('invited')]);
    mockSession('ready', principal);
    const onEntered = vi.fn();

    renderBoundary(onEntered);

    expect(
      await screen.findByRole('heading', { name: 'You’re invited to this room' }),
    ).toBeVisible();
    expect(screen.getByRole('link', { name: 'Open my rooms' })).toHaveAttribute('href', '/rooms');
    expect(screen.queryByRole('button', { name: 'Try again' })).not.toBeInTheDocument();
    expect(onEntered).not.toHaveBeenCalled();
    expect(enter).not.toHaveBeenCalled();
  });

  it('找不到的房间说清楚进不了，不给没用的重试', async () => {
    const enter = vi.fn(() =>
      Promise.resolve(err({ code: 'lobby.entry_not_found', retryable: false })),
    );
    mockServices(enter);
    mockSession('ready', principal);

    renderBoundary(vi.fn());

    expect(
      await screen.findByRole('heading', { name: 'This room isn’t open to you' }),
    ).toBeVisible();
    expect(screen.getByText(/ask its owner to invite you/u)).toBeVisible();
    expect(screen.queryByRole('button', { name: 'Try again' })).not.toBeInTheDocument();
    expect(screen.getByRole('link', { name: 'Back to rooms' })).toHaveAttribute('href', '/rooms');
    expect(enter).toHaveBeenCalledWith(catalogId);
  });
});

const privateMatrixRoomId = '!private:matrix.test';

function privateRoom(status: 'invited' | 'joined'): PrivateRoom {
  return {
    catalogId,
    description: '',
    matrixRoomId: privateMatrixRoomId,
    members: [
      {
        permissions: { capabilities: ['view'] },
        principalId: principal.principalId,
        status,
      },
    ],
    name: 'Research lab',
    ownerPrincipalId: '0198b601-77a3-74f1-b4f4-940f291951ba',
    retentionDays: null,
    roomInstanceId: '0198b601-77a3-74f1-b4f4-940f291951bb',
    status: 'active',
    version: 1,
  } as unknown as PrivateRoom;
}

function mockServices(enter: ReturnType<typeof vi.fn>, rooms: readonly PrivateRoom[] = []) {
  const list = vi.fn(() => Promise.resolve(ok(rooms)));
  vi.mocked(useAppServices).mockReturnValue({
    lobbyEntry: { enter },
    privateRooms: { list },
  } as unknown as ReturnType<typeof useAppServices>);
  return list;
}

const principal = {
  displayName: 'Alice',
  locale: 'en',
  matrixUserId: '@alice:matrix.test',
  principalId: '0198b601-77a3-74f1-b4f4-940f291951b9',
};

function mockSession(value: string, sessionPrincipal: typeof principal | null) {
  vi.mocked(useSession).mockReturnValue({
    snapshot: { context: { principal: sessionPrincipal }, value },
  } as unknown as ReturnType<typeof useSession>);
}

function renderBoundary(
  onEntered: (target: PublicLobbyRouteTarget) => void,
  onConnectionRequired: () => void = vi.fn(),
) {
  const queryClient = new QueryClient({
    defaultOptions: { queries: { retry: false } },
  });
  return render(
    <I18nextProvider i18n={i18n}>
      <QueryClientProvider client={queryClient}>
        <PublicLobbyEntryBoundary
          catalogId={catalogId}
          onConnectionRequired={onConnectionRequired}
          onEntered={onEntered}
        />
      </QueryClientProvider>
    </I18nextProvider>,
  );
}
