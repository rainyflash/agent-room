// @vitest-environment jsdom
import '@testing-library/jest-dom/vitest';
import { QueryClient, QueryClientProvider } from '@tanstack/react-query';
import { cleanup, fireEvent, render, screen, waitFor } from '@testing-library/react';
import { I18nextProvider } from 'react-i18next';
import { afterEach, beforeAll, beforeEach, describe, expect, it, vi } from 'vitest';

import { AppServicesProvider, type AppServices } from '@/app/app-services';
import {
  forgetInviteSecret,
  readInviteSecret,
  stashInviteSecret,
} from '@/features/private-rooms/adapters/browser-invite-secret';
import type { PrivateRoom, PrivateRoomGateway } from '@/features/private-rooms/domain/private-room';
import type { PrivateRoomInviteGateway } from '@/features/private-rooms/domain/private-room-invite-link';
import { i18n, initializeI18n } from '@/shared/i18n/i18n';
import { err, ok } from '@/shared/result';

import { JoinInvitePage } from './join-invite-page';

const SECRET = '0123456789abcdefghjkmnpqrs';
const CATALOG = '0198b601-77a4-7f41-b4f4-940f291951c0';

const session = vi.hoisted((): { value: string; principal: object | null } => ({
  value: 'ready',
  principal: { principalId: '0198b601-77a1-7bb8-83eb-a8fe68c97e43' },
}));
const enter = vi.hoisted(() => vi.fn());

vi.mock('@/features/session/ui/session-provider', () => ({
  useSession: () => ({
    snapshot: { context: { principal: session.principal }, value: session.value },
  }),
}));
vi.mock('@/features/room-directory/ui/use-rooms', () => ({
  useRoomCommands: () => ({ enter }),
}));

const room = { catalogId: CATALOG } as PrivateRoom;

function gateways() {
  const invites = {
    inviteLink: vi.fn(),
    createInviteLink: vi.fn(),
    disableInviteLink: vi.fn(),
    previewInvite: vi.fn<PrivateRoomInviteGateway['previewInvite']>(() =>
      Promise.resolve(
        ok({
          catalogId: CATALOG,
          expiresAtUnixMs: Date.UTC(2026, 9, 21),
          inviterDisplayName: 'Ada',
          membership: 'absent',
          permissions: { capabilities: ['view', 'speak', 'automate'] },
          roomName: 'Weekly review',
        }),
      ),
    ),
    redeemInvite: vi.fn<PrivateRoomInviteGateway['redeemInvite']>(() =>
      Promise.resolve(ok({ catalogId: CATALOG, membership: 'invited' })),
    ),
  } satisfies PrivateRoomInviteGateway;
  const rooms = { inspect: vi.fn(() => Promise.resolve(ok(room))) };
  return { invites, rooms };
}

function renderPage(
  { invites, rooms }: ReturnType<typeof gateways>,
  onConnectionRequired = vi.fn(),
) {
  const services = {
    privateRoomInvites: invites,
    privateRooms: rooms as unknown as PrivateRoomGateway,
  } as unknown as AppServices;
  render(
    <I18nextProvider i18n={i18n}>
      <QueryClientProvider client={new QueryClient()}>
        <AppServicesProvider services={services}>
          <JoinInvitePage onConnectionRequired={onConnectionRequired} />
        </AppServicesProvider>
      </QueryClientProvider>
    </I18nextProvider>,
  );
  return onConnectionRequired;
}

function openLink(): void {
  stashInviteSecret(
    { hash: `#${SECRET}`, pathname: '/join', search: '' },
    { replaceState: () => undefined, state: null },
  );
}

beforeAll(async () => {
  await initializeI18n(window.localStorage, ['en']);
});

beforeEach(() => {
  session.value = 'ready';
  session.principal = { principalId: '0198b601-77a1-7bb8-83eb-a8fe68c97e43' };
  enter.mockReset();
  enter.mockResolvedValue(ok(room));
  forgetInviteSecret();
});

afterEach(cleanup);

describe('打开邀请链接', () => {
  it('没登录先去登录，秘密留在这个标签页里', () => {
    openLink();
    session.value = 'unauthenticated';
    session.principal = null;
    const services = gateways();

    const onConnectionRequired = renderPage(services);

    expect(onConnectionRequired).toHaveBeenCalled();
    expect(services.invites.previewInvite).not.toHaveBeenCalled();
    expect(readInviteSecret()).toBe(SECRET);
  });

  it('看清是谁邀请的，点加入以后受邀、接受邀请进房间，再忘掉秘密', async () => {
    openLink();
    const services = gateways();
    renderPage(services);

    expect(
      await screen.findByRole('heading', { name: 'Ada invited you to “Weekly review”' }),
    ).toBeVisible();
    expect(services.invites.previewInvite).toHaveBeenCalledWith(SECRET);
    fireEvent.click(screen.getByRole('button', { name: 'Join' }));

    await waitFor(() => {
      expect(enter).toHaveBeenCalledWith(room, true);
    });
    expect(services.invites.redeemInvite).toHaveBeenCalledWith(SECRET);
    expect(services.rooms.inspect).toHaveBeenCalledWith(CATALOG);
    expect(readInviteSecret()).toBeNull();
  });

  it('已经在房间里的直接打开，不再接受一次', async () => {
    openLink();
    const services = gateways();
    services.invites.redeemInvite.mockResolvedValue(
      ok({ catalogId: CATALOG, membership: 'joined' }),
    );
    renderPage(services);

    fireEvent.click(await screen.findByRole('button', { name: 'Join' }));

    await waitFor(() => {
      expect(enter).toHaveBeenCalledWith(room, false);
    });
  });

  it('链接失效时说清楚、只给“回到房间”，秘密也不留了', async () => {
    openLink();
    const services = gateways();
    services.invites.previewInvite.mockResolvedValue(
      err({ code: 'invite_link.not_found', retryable: false }),
    );
    renderPage(services);

    expect(await screen.findByRole('heading', { name: 'This link no longer works' })).toBeVisible();
    expect(screen.queryByRole('button', { name: 'Try again' })).toBeNull();
    await waitFor(() => {
      expect(readInviteSecret()).toBeNull();
    });
  });

  it('加入时服务器暂时不通：说没成、能再试，秘密留着', async () => {
    openLink();
    const services = gateways();
    services.invites.redeemInvite.mockResolvedValue(
      err({ code: 'invite_link.dependency_unavailable', retryable: true }),
    );
    renderPage(services);

    fireEvent.click(await screen.findByRole('button', { name: 'Join' }));

    expect(
      await screen.findByRole('heading', { name: 'The invitation didn’t open' }),
    ).toBeVisible();
    expect(screen.getByRole('button', { name: 'Try again' })).toBeVisible();
    expect(readInviteSecret()).toBe(SECRET);
    expect(enter).not.toHaveBeenCalled();
  });

  it('没有秘密时说链接不完整，不去问服务器', () => {
    const services = gateways();
    renderPage(services);
    expect(screen.getByRole('heading', { name: 'This invite link is incomplete' })).toBeVisible();
    expect(services.invites.previewInvite).not.toHaveBeenCalled();
  });
});
