// @vitest-environment jsdom
import '@testing-library/jest-dom/vitest';
import { QueryClient, QueryClientProvider } from '@tanstack/react-query';
import { cleanup, fireEvent, render, screen, waitFor } from '@testing-library/react';
import { I18nextProvider } from 'react-i18next';
import { afterEach, beforeAll, describe, expect, it, vi } from 'vitest';

import type { JoinCode, PrivateRoomGateway } from '@/features/private-rooms/domain/private-room';
import type { SubRoomAccess } from '@/features/private-rooms/domain/sub-rooms';
import { i18n, initializeI18n } from '@/shared/i18n/i18n';
import { err, ok } from '@/shared/result';

import { SubRoomNetworkInvite } from './sub-room-network-invite';

const PARENT = '0198b601-77a1-7bb8-83eb-a8fe68c97e46';
const SUB = '0198b601-77a1-7bb8-83eb-a8fe68c97e52';
const GUIDE = 'https://app.agent-room.test/agents.txt';
const CODE = 'K7P3-Q9XW-2DMA';
const CREATED = Date.UTC(2026, 10, 21);

beforeAll(async () => {
  await initializeI18n(window.localStorage, ['en']);
});
afterEach(() => {
  cleanup();
  vi.unstubAllGlobals();
});

/** 父房间的口令：有、看得到，或者还没有。子房间没有口令：拿子房间号来问口令的一律回 409。 */
function gateway(parentCode: 'shown' | 'none') {
  const parentOnly = () => Promise.resolve(err({ code: 'sub_room.parent_only', retryable: false }));
  const unavailable = () => Promise.resolve(err({ code: 'test.unavailable', retryable: false }));
  let code: JoinCode | null =
    parentCode === 'shown' ? { code: CODE, createdAtUnixMs: CREATED } : null;
  const agentAccess = vi.fn<PrivateRoomGateway['agentAccess']>((catalogId) =>
    catalogId === PARENT
      ? Promise.resolve(
          ok({
            agents: [],
            joinCode: code === null ? null : { createdAtUnixMs: code.createdAtUnixMs },
          }),
        )
      : parentOnly(),
  );
  const revealJoinCode = vi.fn<PrivateRoomGateway['revealJoinCode']>((catalogId) =>
    catalogId === PARENT && code !== null ? Promise.resolve(ok(code)) : parentOnly(),
  );
  const generateJoinCode = vi.fn<PrivateRoomGateway['generateJoinCode']>((catalogId) => {
    if (catalogId !== PARENT) return parentOnly();
    code = { code: CODE, createdAtUnixMs: CREATED + 60_000 };
    return Promise.resolve(ok(code));
  });
  const agentKnocks = vi.fn<PrivateRoomGateway['agentKnocks']>(() => Promise.resolve(ok([])));
  const value: PrivateRoomGateway = {
    accept: unavailable,
    admitKnock: unavailable,
    agentAccess,
    agentKnocks,
    archive: unavailable,
    ban: unavailable,
    create: unavailable,
    decline: unavailable,
    declineKnock: unavailable,
    disableJoinCode: unavailable,
    generateJoinCode,
    inspect: unavailable,
    invite: unavailable,
    leave: unavailable,
    list: () => Promise.resolve(ok([])),
    origin: unavailable,
    people: unavailable,
    remove: unavailable,
    removeCodeAgent: unavailable,
    rename: unavailable,
    revealJoinCode,
    transferOwnership: unavailable,
    unban: unavailable,
    updatePermissions: unavailable,
  };
  return { agentAccess, agentKnocks, generateJoinCode, revealJoinCode, value };
}

function renderInvite(rooms: PrivateRoomGateway, access: SubRoomAccess, manages = true) {
  const client = new QueryClient({ defaultOptions: { queries: { retry: false } } });
  render(
    <QueryClientProvider client={client}>
      <I18nextProvider i18n={i18n}>
        <SubRoomNetworkInvite
          guide={GUIDE}
          room={{
            access,
            catalogId: SUB,
            manages,
            name: 'release notes',
            parent: { catalogId: PARENT, name: 'new game dev' },
          }}
          rooms={rooms}
        />
      </I18nextProvider>
    </QueryClientProvider>,
  );
}

describe('子房间里请网络 Agent（agent-room-ops#112）', () => {
  it('没有门：先凭父房间的口令进父房间，话的最后叫它拿子房间号进这一间', async () => {
    const rooms = gateway('shown');
    renderInvite(rooms.value, 'open');

    const message = await screen.findByText(
      /enter the Agent Room private room “new game dev” with the code K7P3-Q9XW-2DMA/u,
    );
    expect(message).toHaveTextContent(
      `Once you’re in “new game dev”, go on into its sub-room “release notes” with the room number ${SUB}`,
    );
    expect(message).toHaveTextContent(`knock with the room number ${PARENT} instead`);
    expect(screen.getByText(/“release notes” is a sub-room of “new game dev”/u)).toBeVisible();
    // 口令只问父房间的；子房间没有口令，也不列它的敲门。
    expect(rooms.agentAccess).not.toHaveBeenCalledWith(SUB);
    expect(rooms.revealJoinCode).not.toHaveBeenCalledWith(SUB, expect.anything());
    expect(rooms.agentKnocks).not.toHaveBeenCalledWith(SUB);
  });

  it('父房间还没有口令：生成的是父房间的口令，不再去要子房间的（以前回 409）', async () => {
    const rooms = gateway('none');
    vi.stubGlobal('navigator', { clipboard: { writeText: vi.fn(() => Promise.resolve()) } });
    renderInvite(rooms.value, 'open');

    const button = await screen.findByRole('button', { name: 'Create code and copy' });
    await waitFor(() => {
      expect(button).toBeEnabled();
    });
    fireEvent.click(button);

    expect(
      await screen.findByText(/with the code K7P3-Q9XW-2DMA[\s\S]*its sub-room “release notes”/u),
    ).toBeVisible();
    expect(rooms.generateJoinCode).toHaveBeenCalledTimes(1);
    expect(rooms.generateJoinCode).toHaveBeenCalledWith(PARENT);
  });

  it('要批准的门：话里叫它敲子房间的门；管得了这一间的人在这里看到敲这一间的门的 Agent', async () => {
    const rooms = gateway('shown');
    renderInvite(rooms.value, 'request');

    const message = await screen.findByText(/with the code K7P3-Q9XW-2DMA/u);
    expect(message).toHaveTextContent(
      `knock on its sub-room “release notes” with the room number ${SUB} and wait for someone to let you in`,
    );
    expect(
      await screen.findByText(
        'When your agent knocks on “release notes”, it shows up here for you to let in.',
      ),
    ).toBeVisible();
    expect(rooms.agentKnocks).toHaveBeenCalledWith(SUB);
  });

  it('要批准的门、看的人管不了这一间：说要管这一间的人放行，不去问这一间的敲门', async () => {
    const rooms = gateway('shown');
    renderInvite(rooms.value, 'request', false);

    expect(
      await screen.findByText(/someone who manages this sub-room has to let it in/u),
    ).toBeVisible();
    expect(rooms.agentKnocks).not.toHaveBeenCalledWith(SUB);
  });

  it('只能请进来：网络 Agent 拿房间号进不来，只说这一句，什么都不去问', () => {
    const rooms = gateway('shown');
    renderInvite(rooms.value, 'invite_only');

    expect(
      screen.getByText(
        '“release notes” is invite-only, so a network agent can’t come in with its room number.',
      ),
    ).toBeVisible();
    expect(screen.queryByRole('button')).toBeNull();
    expect(rooms.agentAccess).not.toHaveBeenCalled();
  });
});
