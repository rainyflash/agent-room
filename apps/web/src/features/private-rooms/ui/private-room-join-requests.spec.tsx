// @vitest-environment jsdom
import '@testing-library/jest-dom/vitest';
import { QueryClient, QueryClientProvider } from '@tanstack/react-query';
import { cleanup, fireEvent, render, screen, waitFor } from '@testing-library/react';
import { I18nextProvider } from 'react-i18next';
import { afterEach, beforeAll, describe, expect, it, vi } from 'vitest';

import type { PrivateRoomListing } from '@/features/private-rooms/domain/private-room-listing';
import type {
  PendingJoinRequest,
  PublicRoomJoinGateway,
} from '@/features/private-rooms/domain/public-room-join';
import { i18n, initializeI18n } from '@/shared/i18n/i18n';
import { err, ok } from '@/shared/result';

import { PrivateRoomJoinRequests } from './private-room-join-requests';

const CATALOG = '0198b601-77a1-7bb8-83eb-a8fe68c97e46';
const LIN = '0198b601-77a1-7bb8-83eb-a8fe68c97e50';
const MO = '0198b601-77a1-7bb8-83eb-a8fe68c97e51';

const requestListing: PrivateRoomListing = {
  allowNetworkAgents: true,
  endToEndEncrypted: false,
  firstListedAtUnixMs: 1_780_000_000_000,
  joinMode: 'request',
  listed: true,
  memberPermissions: { capabilities: ['view', 'speak', 'automate'] },
  platformHold: null,
  shareCode: 'aB3xYz',
  shareUrl: 'https://agentroom.test/r/aB3xYz',
};

const pending: readonly PendingJoinRequest[] = [
  {
    displayName: 'Lin',
    message: 'I run the weekly eval review.\nCan I bring my agent?',
    principalId: LIN,
    requestedAtUnixMs: 1_780_000_000_000,
  },
  { displayName: 'Mo', message: '', principalId: MO, requestedAtUnixMs: 1_780_000_600_000 },
];

beforeAll(async () => {
  await initializeI18n(window.localStorage, ['en']);
});
afterEach(() => {
  cleanup();
});

function gateway(initial: readonly PendingJoinRequest[]) {
  let current = initial;
  const answerJoinRequest = vi.fn<PublicRoomJoinGateway['answerJoinRequest']>(
    (_catalogId, principalId) => {
      current = current.filter((request) => request.principalId !== principalId);
      return Promise.resolve(ok(undefined));
    },
  );
  const value: PublicRoomJoinGateway = {
    answerJoinRequest,
    joinRequests: () => Promise.resolve(ok(current)),
  };
  return { answerJoinRequest, value };
}

function renderRequests(joins: PublicRoomJoinGateway, listing: PrivateRoomListing | null) {
  const client = new QueryClient({ defaultOptions: { queries: { retry: false } } });
  return render(
    <QueryClientProvider client={client}>
      <I18nextProvider i18n={i18n}>
        <PrivateRoomJoinRequests catalogId={CATALOG} joins={joins} listing={listing} />
      </I18nextProvider>
    </QueryClientProvider>,
  );
}

describe('PrivateRoomJoinRequests', () => {
  it('列出在等的申请和留的话，批准以后从列表里拿掉', async () => {
    const joins = gateway(pending);
    renderRequests(joins.value, requestListing);

    expect(await screen.findByText('Lin')).toBeInTheDocument();
    expect(screen.getByText(/I run the weekly eval review\./u)).toBeInTheDocument();
    expect(screen.getByText('Mo')).toBeInTheDocument();

    fireEvent.click(screen.getByRole('button', { name: 'Approve Lin' }));
    await waitFor(() => {
      expect(screen.queryByText('Lin')).not.toBeInTheDocument();
    });
    expect(joins.answerJoinRequest).toHaveBeenCalledWith(CATALOG, LIN, 'approve');

    fireEvent.click(screen.getByRole('button', { name: 'Reject Mo' }));
    expect(await screen.findByText('No one is waiting.')).toBeInTheDocument();
    expect(joins.answerJoinRequest).toHaveBeenLastCalledWith(CATALOG, MO, 'reject');
  });

  it('被封禁的人批准不了：说先在成员里解封', async () => {
    const joins = gateway(pending);
    joins.answerJoinRequest.mockResolvedValueOnce(
      err({ code: 'room_join.banned', retryable: false }),
    );
    renderRequests(joins.value, requestListing);

    fireEvent.click(await screen.findByRole('button', { name: 'Approve Lin' }));
    expect(
      await screen.findByText('This person is banned here. Unban them in the member list first.'),
    ).toBeInTheDocument();
    expect(screen.getByText('Lin')).toBeInTheDocument();
  });

  it('不用申请的房间没人申请时整节都不出现，有留下的申请照样列出', async () => {
    const empty = gateway([]);
    const { unmount } = renderRequests(empty.value, { ...requestListing, joinMode: 'open' });
    await waitFor(() => {
      expect(screen.queryByRole('heading', { name: 'Requests to join' })).not.toBeInTheDocument();
    });
    unmount();

    // 改成直接进以前留下的申请还在等，照样给房主处理。
    renderRequests(gateway(pending).value, { ...requestListing, joinMode: 'open' });
    expect(await screen.findByRole('heading', { name: 'Requests to join' })).toBeInTheDocument();
  });
});
