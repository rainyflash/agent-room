// @vitest-environment jsdom
import '@testing-library/jest-dom/vitest';
import { QueryClient, QueryClientProvider } from '@tanstack/react-query';
import { cleanup, fireEvent, render, screen, waitFor, within } from '@testing-library/react';
import { I18nextProvider } from 'react-i18next';
import { afterEach, beforeAll, describe, expect, it, vi } from 'vitest';

import type { PrivateRoom } from '@/features/private-rooms/domain/private-room';
import type {
  PrivateRoomListing,
  PrivateRoomListingGateway,
  PrivateRoomListingSettings,
} from '@/features/private-rooms/domain/private-room-listing';
import type {
  ProjectRoomGateway,
  RoomRepositoryBinding,
} from '@/features/private-rooms/domain/project-rooms';
import type {
  RoomLiveView,
  RoomLiveViewGateway,
} from '@/features/private-rooms/domain/room-live-view';
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

function renderSettings(
  listings: PrivateRoomListingGateway,
  principalId = OWNER,
  projects?: ProjectRoomGateway,
  liveViews?: RoomLiveViewGateway,
) {
  const client = new QueryClient({ defaultOptions: { queries: { retry: false } } });
  return render(
    <QueryClientProvider client={client}>
      <I18nextProvider i18n={i18n}>
        <PrivateRoomPublicSettings
          listings={listings}
          liveViews={liveViews}
          principalId={principalId}
          projects={projects}
          room={room}
        />
      </I18nextProvider>
    </QueryClientProvider>,
  );
}

const liveOff: RoomLiveView = {
  canChange: true,
  enabled: false,
  enabledAtUnixMs: null,
  live: false,
  unmetConditions: [],
};

function liveViews(view: RoomLiveView = liveOff) {
  const liveView = vi.fn<RoomLiveViewGateway['liveView']>(() => Promise.resolve(ok(view)));
  const updateLiveView = vi.fn<RoomLiveViewGateway['updateLiveView']>(() =>
    Promise.resolve(ok(view)),
  );
  return { liveView, updateLiveView };
}

const boundRepository: RoomRepositoryBinding = {
  agentsLine: 'Agent Room: this project’s room is https://agentroom.test/r/aB3xYz.',
  badgeMarkdown:
    '[![Agent Room](https://agentroom.test/gh/o/r/badge.svg)](https://agentroom.test/r/aB3xYz)',
  repository: 'rainyflash/agent-room',
  repositoryUrl: 'https://github.com/rainyflash/agent-room',
  shareCode: 'aB3xYz',
  shareUrl: 'https://agentroom.test/r/aB3xYz',
  verifiedAtUnixMs: 1_790_000_600_000,
  verifiedByAccountId: OWNER,
  verifiedFile: 'README.md',
};

function projectRooms(
  binding: RoomRepositoryBinding | null,
  read = vi.fn<ProjectRoomGateway['binding']>(() => Promise.resolve(ok(binding))),
): ProjectRoomGateway {
  const unused = () => Promise.resolve(err({ code: 'project_room.unused', retryable: false }));
  return {
    binding: read,
    cancelClaim: unused,
    checkBinding: unused,
    checkClaim: unused,
    claim: unused,
    claims: () => Promise.resolve(ok([])),
    inspectClaim: unused,
    roomRepository: unused,
    unbind: unused,
  };
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

    expect(
      await screen.findByText(
        /Each account can have only one public room that isn’t linked to a repository/u,
      ),
    ).toBeInTheDocument();
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

  it('“GitHub 仓库”一组只有房主、只在公开着的房间里看得到', async () => {
    const read = vi.fn<ProjectRoomGateway['binding']>(() => Promise.resolve(ok(null)));
    const projects = projectRooms(null, read);
    const owner = renderSettings(gateway(publicListing).value, OWNER, projects);
    expect(await screen.findByRole('heading', { name: 'GitHub repository' })).toBeVisible();
    expect(await screen.findByRole('textbox', { name: 'Repository' })).toBeVisible();
    owner.unmount();

    const member = renderSettings(gateway(publicListing).value, MEMBER, projects);
    await screen.findByText('Only the owner can change these settings.');
    expect(screen.queryByRole('heading', { name: 'GitHub repository' })).toBeNull();
    member.unmount();

    renderSettings(gateway(privateListing).value, OWNER, projects);
    await screen.findByRole('group', { name: 'Who can find this room' });
    expect(screen.queryByRole('heading', { name: 'GitHub repository' })).toBeNull();
    expect(read).toHaveBeenCalledTimes(1);
  });

  it('绑着仓库的房间不给换分享码，说链接写在仓库里', async () => {
    renderSettings(gateway(publicListing).value, OWNER, projectRooms(boundRepository));

    expect(
      await screen.findByText(
        'A room linked to a repository can’t change its share link: the link is written in the repository.',
      ),
    ).toBeVisible();
    expect(screen.queryByRole('button', { name: 'New share code' })).toBeNull();
    expect(screen.getByLabelText('Share link')).toHaveTextContent(
      'https://agentroom.test/r/aB3xYz',
    );
  });

  it('换码时服务器说房间刚绑上仓库（409）：照新的说法讲', async () => {
    const listings = gateway(publicListing);
    listings.changeShareCode.mockResolvedValueOnce(
      err({ code: 'room_listing.repository_bound', retryable: false }),
    );
    renderSettings(listings.value);

    fireEvent.click(await screen.findByRole('button', { name: 'New share code' }));
    fireEvent.click(screen.getByRole('button', { name: 'Change it' }));

    expect(
      await screen.findByText(
        'A room linked to a repository can’t change its share link: the link is written in the repository.',
      ),
    ).toBeVisible();
    expect(screen.queryByText('room_listing.repository_bound')).toBeNull();
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

describe('“公开”一节里的“分享页放实况”', () => {
  const liveRegion = () => screen.queryByRole('region', { name: 'Live view on the share page' });

  it('公开着的房间里房主和别的成员都看得到，房主那里排在分享链接后面、GitHub 仓库前面', async () => {
    const live = liveViews();
    const owner = renderSettings(gateway(publicListing).value, OWNER, projectRooms(null), live);
    const region = await screen.findByRole('region', { name: 'Live view on the share page' });
    const share = screen.getByRole('heading', { name: 'Share link' });
    const repository = await screen.findByRole('heading', { name: 'GitHub repository' });
    expect(share.compareDocumentPosition(region) & Node.DOCUMENT_POSITION_FOLLOWING).toBeTruthy();
    expect(
      region.compareDocumentPosition(repository) & Node.DOCUMENT_POSITION_FOLLOWING,
    ).toBeTruthy();
    owner.unmount();

    // 别的成员（服务器说他不能开关）：照样看得到开没开，开关只给房主和管理员。
    renderSettings(
      gateway(publicListing).value,
      MEMBER,
      undefined,
      liveViews({ ...liveOff, canChange: false }),
    );
    const memberRegion = await screen.findByRole('region', {
      name: 'Live view on the share page',
    });
    expect(await within(memberRegion).findByText('Off.')).toBeInTheDocument();
    expect(within(memberRegion).queryByRole('switch')).toBeNull();
    expect(memberRegion).toHaveTextContent('Only the owner and admins can turn it on or off.');
  });

  it('私密的房间、没有这项服务时都没有这一组，也不去问', async () => {
    const live = liveViews();
    const privateRoom = renderSettings(gateway(privateListing).value, OWNER, undefined, live);
    await screen.findByRole('group', { name: 'Who can find this room' });
    expect(liveRegion()).toBeNull();
    expect(live.liveView).not.toHaveBeenCalled();
    privateRoom.unmount();

    renderSettings(gateway(publicListing).value);
    await screen.findByLabelText('Share link');
    expect(liveRegion()).toBeNull();
  });

  it('存好公开设置以后再问一次实况：换了加入方式，服务器可能已经把它关了', async () => {
    const live = liveViews({
      ...liveOff,
      enabled: true,
      enabledAtUnixMs: 1_790_000_000_000,
      live: true,
    });
    renderSettings(gateway(publicListing).value, OWNER, undefined, live);
    expect(
      await screen.findByRole('switch', { name: 'Show what’s said here on the share page' }),
    ).toBeChecked();
    expect(live.liveView).toHaveBeenCalledTimes(1);

    live.liveView.mockResolvedValue(ok({ ...liveOff, unmetConditions: ['join_mode'] }));
    fireEvent.click(screen.getByRole('radio', { name: /Ask to join/u }));
    fireEvent.click(screen.getByRole('button', { name: 'Save' }));
    expect(await screen.findByText('Saved.')).toBeInTheDocument();

    await waitFor(() => {
      expect(
        screen.getByRole('switch', { name: 'Show what’s said here on the share page' }),
      ).not.toBeChecked();
    });
    expect(live.liveView).toHaveBeenCalledTimes(2);
    expect(
      screen.getByText(
        'Only rooms people can join directly can show a live view. Set How people join to Join directly first.',
      ),
    ).toBeInTheDocument();
  });

  it('设成公开的确认框说实况不会自己打开：不加密、直接进时才说', async () => {
    const plainPrivate: PrivateRoomListing = { ...privateListing, endToEndEncrypted: false };
    const live = liveViews();
    const plain = renderSettings(gateway(plainPrivate).value, OWNER, undefined, live);
    fireEvent.click(await screen.findByRole('radio', { name: /Public/u }));
    fireEvent.click(screen.getByRole('button', { name: 'Save' }));
    const later =
      'The live view on the share page doesn’t turn on by itself. Once the room is public, you can turn it on in this section.';
    expect(screen.getByText(later)).toBeInTheDocument();
    // 改成要申请：开不了，就不说。
    fireEvent.click(screen.getByRole('button', { name: 'Cancel' }));
    fireEvent.click(screen.getByRole('radio', { name: /Ask to join/u }));
    fireEvent.click(screen.getByRole('button', { name: 'Save' }));
    expect(screen.queryByText(later)).toBeNull();
    plain.unmount();

    // 加密的房间公开以后也加密，开不了实况。
    renderSettings(gateway(privateListing).value, OWNER, undefined, live);
    fireEvent.click(await screen.findByRole('radio', { name: /Public/u }));
    fireEvent.click(screen.getByRole('button', { name: 'Save' }));
    expect(screen.getByText(/It stays encrypted/u)).toBeInTheDocument();
    expect(screen.queryByText(later)).toBeNull();
  });
});
