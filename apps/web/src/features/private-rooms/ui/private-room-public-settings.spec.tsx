// @vitest-environment jsdom
import '@testing-library/jest-dom/vitest';
import { QueryClient, QueryClientProvider } from '@tanstack/react-query';
import { cleanup, fireEvent, render, screen, within } from '@testing-library/react';
import { I18nextProvider } from 'react-i18next';
import { afterEach, beforeAll, describe, expect, it, vi } from 'vitest';

import type { PrivateRoom } from '@/features/private-rooms/domain/private-room';
import type {
  PrivateRoomListing,
  PrivateRoomListingGateway,
  PrivateRoomListingSettings,
} from '@/features/private-rooms/domain/private-room-listing';
import { i18n, initializeI18n } from '@/shared/i18n/i18n';
import { err, ok } from '@/shared/result';

import { PrivateRoomPublicSettings } from './private-room-public-settings';

const CATALOG = '0198b601-77a1-7bb8-83eb-a8fe68c97e46';
const OWNER = '0198b601-77a1-7bb8-83eb-a8fe68c97e42';
const MEMBER = '0198b601-77a1-7bb8-83eb-a8fe68c97e43';

const room: PrivateRoom = {
  catalogId: CATALOG,
  description: '',
  matrixRoomId: '!book:matrix.test',
  members: [
    {
      permissions: { capabilities: ['view', 'speak', 'invite', 'manage', 'automate'] },
      principalId: OWNER,
      status: 'joined',
    },
    { permissions: { capabilities: ['view', 'speak'] }, principalId: MEMBER, status: 'joined' },
  ],
  name: 'Book club',
  ownerPrincipalId: OWNER,
  retentionDays: null,
  roomInstanceId: '0198b601-77a1-7bb8-83eb-a8fe68c97e47',
  status: 'active',
  version: 1,
};

const privateListing: PrivateRoomListing = {
  allowNetworkAgents: true,
  endToEndEncrypted: true,
  firstListedAtUnixMs: null,
  joinMode: 'open',
  listed: false,
  memberPermissions: { capabilities: ['view', 'speak', 'automate'] },
  platformHold: null,
  shareCode: null,
  shareUrl: null,
};

const publicListing: PrivateRoomListing = {
  ...privateListing,
  endToEndEncrypted: false,
  firstListedAtUnixMs: 1_780_000_000_000,
  listed: true,
  shareCode: 'aB3xYz',
  shareUrl: 'https://agentroom.test/r/aB3xYz',
};

beforeAll(async () => {
  await initializeI18n(window.localStorage, ['en']);
});
afterEach(() => {
  cleanup();
});

function gateway(initial: PrivateRoomListing) {
  let current = initial;
  const updateListing = vi.fn<PrivateRoomListingGateway['updateListing']>(
    (_catalogId: string, settings: PrivateRoomListingSettings) => {
      current = { ...current, ...settings };
      if (settings.listed && current.shareCode === null) {
        current = { ...current, shareCode: 'aB3xYz', shareUrl: 'https://agentroom.test/r/aB3xYz' };
      }
      return Promise.resolve(ok(current));
    },
  );
  const changeShareCode = vi.fn<PrivateRoomListingGateway['changeShareCode']>(() => {
    current = { ...current, shareCode: 'Zz9Zz9', shareUrl: 'https://agentroom.test/r/Zz9Zz9' };
    return Promise.resolve(ok(current));
  });
  const value: PrivateRoomListingGateway = {
    changeShareCode,
    listing: () => Promise.resolve(ok(current)),
    updateListing,
  };
  return { changeShareCode, updateListing, value };
}

function renderSettings(listings: PrivateRoomListingGateway, principalId = OWNER) {
  const client = new QueryClient({ defaultOptions: { queries: { retry: false } } });
  return render(
    <QueryClientProvider client={client}>
      <I18nextProvider i18n={i18n}>
        <PrivateRoomPublicSettings listings={listings} principalId={principalId} room={room} />
      </I18nextProvider>
    </QueryClientProvider>,
  );
}

describe('PrivateRoomPublicSettings', () => {
  it('房主把私密房间设成公开：先确认，说清楚谁看得到、加密照旧，保存后给分享链接', async () => {
    const listings = gateway(privateListing);
    renderSettings(listings.value);

    const visibility = await screen.findByRole('group', { name: 'Who can find this room' });
    expect(within(visibility).getByRole('radio', { name: /Private/u })).toBeChecked();
    expect(screen.getByRole('button', { name: 'Save' })).toBeDisabled();
    expect(screen.queryByRole('group', { name: 'How people join' })).not.toBeInTheDocument();

    fireEvent.click(within(visibility).getByRole('radio', { name: /Public/u }));
    fireEvent.click(screen.getByRole('radio', { name: /Ask to join/u }));
    // 新成员拿不到管理。
    expect(screen.getByRole('checkbox', { name: /Manage/u })).toBeDisabled();
    fireEvent.click(screen.getByRole('button', { name: 'Save' }));

    expect(screen.getByText('Make this room public?')).toBeInTheDocument();
    expect(
      screen.getAllByText(/Without signing in, anyone can see on Explore/u).length,
    ).toBeGreaterThan(0);
    expect(screen.getByText(/It stays encrypted/u)).toBeInTheDocument();
    expect(listings.updateListing).not.toHaveBeenCalled();

    fireEvent.click(screen.getByRole('button', { name: 'Make public' }));
    expect(await screen.findByText('Saved.')).toBeInTheDocument();
    expect(listings.updateListing).toHaveBeenCalledWith(CATALOG, {
      allowNetworkAgents: true,
      joinMode: 'request',
      listed: true,
      memberPermissions: { capabilities: ['view', 'speak', 'automate'] },
    });
    expect(await screen.findByLabelText('Share link')).toHaveTextContent(
      'https://agentroom.test/r/aB3xYz',
    );
  });

  it('公开着改加入方式不用确认；改回私密要确认，说加密从那一刻起打开', async () => {
    const listings = gateway(publicListing);
    renderSettings(listings.value);

    fireEvent.click(await screen.findByRole('radio', { name: /Invite only/u }));
    fireEvent.click(screen.getByRole('button', { name: 'Save' }));
    expect(await screen.findByText('Saved.')).toBeInTheDocument();
    expect(listings.updateListing).toHaveBeenLastCalledWith(
      CATALOG,
      expect.objectContaining({ joinMode: 'invite_only', listed: true }),
    );
    // 只能邀请的房间不给分享链接。
    expect(screen.queryByLabelText('Share link')).not.toBeInTheDocument();
    expect(screen.getByText(/Invite-only rooms have no share link/u)).toBeInTheDocument();

    fireEvent.click(screen.getByRole('radio', { name: /Private/u }));
    fireEvent.click(screen.getByRole('button', { name: 'Save' }));
    expect(screen.getByText('Make this room private?')).toBeInTheDocument();
    expect(screen.getByText(/Encryption turns on from now/u)).toBeInTheDocument();
    fireEvent.click(screen.getByRole('button', { name: 'Cancel' }));
    expect(listings.updateListing).toHaveBeenCalledTimes(1);
  });

  it('换分享码要确认一次，换好以后显示新链接', async () => {
    const listings = gateway(publicListing);
    renderSettings(listings.value);

    fireEvent.click(await screen.findByRole('button', { name: 'New share code' }));
    expect(screen.getByText(/The old link stops working right away/u)).toBeInTheDocument();
    fireEvent.click(screen.getByRole('button', { name: 'Change it' }));

    expect(await screen.findByText(/Share code changed/u)).toBeInTheDocument();
    expect(listings.changeShareCode).toHaveBeenCalledWith(CATALOG);
    expect(screen.getByLabelText('Share link')).toHaveTextContent(
      'https://agentroom.test/r/Zz9Zz9',
    );
  });

  it('已经有一间公开房间时说清楚要先把另一间改回私密', async () => {
    const listings = gateway(privateListing);
    listings.updateListing.mockResolvedValueOnce(
      err({ code: 'room_listing.limit_reached', retryable: false }),
    );
    renderSettings(listings.value);

    fireEvent.click(await screen.findByRole('radio', { name: /Public/u }));
    fireEvent.click(screen.getByRole('button', { name: 'Save' }));
    fireEvent.click(screen.getByRole('button', { name: 'Make public' }));

    expect(await screen.findByText(/Each account can have one public room/u)).toBeInTheDocument();
    expect(screen.getByRole('radio', { name: /Public/u })).toBeChecked();
  });

  it('别的成员只看得到现在怎样和分享链接，不能改', async () => {
    renderSettings(gateway(publicListing).value, MEMBER);

    expect(
      await screen.findByText('This room is public. How people join: Join directly.'),
    ).toBeInTheDocument();
    expect(screen.getByText(/This room is not encrypted/u)).toBeInTheDocument();
    expect(screen.getByLabelText('Share link')).toHaveTextContent(
      'https://agentroom.test/r/aB3xYz',
    );
    expect(screen.queryByRole('radio')).not.toBeInTheDocument();
    expect(screen.queryByRole('button', { name: 'New share code' })).not.toBeInTheDocument();
    expect(screen.getByText('Only the owner can change these settings.')).toBeInTheDocument();
  });

  it('平台拿掉了：写原因，不给分享链接', async () => {
    renderSettings(
      gateway({
        ...publicListing,
        platformHold: { heldAtUnixMs: 1_780_000_000_000, reason: 'spam' },
      }).value,
    );

    expect(await screen.findByText('Agent Room took this room off Explore')).toBeInTheDocument();
    expect(screen.getByText(/Reason: spam\./u)).toBeInTheDocument();
    expect(screen.queryByLabelText('Share link')).not.toBeInTheDocument();
  });
});
