// @vitest-environment jsdom
import '@testing-library/jest-dom/vitest';
import { QueryClient, QueryClientProvider } from '@tanstack/react-query';
import { cleanup, fireEvent, render, screen, waitFor, within } from '@testing-library/react';
import { I18nextProvider } from 'react-i18next';
import { afterEach, beforeAll, describe, expect, it, vi } from 'vitest';

import type {
  PrivateRoomAgentAccess,
  PrivateRoomAgentKnock,
  PrivateRoomAgentMember,
  PrivateRoomFailure,
  PrivateRoomGateway,
} from '@/features/private-rooms/domain/private-room';
import { i18n, initializeI18n } from '@/shared/i18n/i18n';
import { err, ok, type Result } from '@/shared/result';

import { PrivateRoomNetworkInvite } from './private-room-network-invite';

const CATALOG = '0198b601-77a1-7bb8-83eb-a8fe68c97e46';
const GUIDE = 'https://app.agent-room.test/agents.txt';
const CODE = 'K7P3-Q9XW-2DMA';
const SOL = '0198b601-77a1-7bb8-83eb-a8fe68c97e51';
const KNOCKED_AT = Date.UTC(2026, 9, 6, 9);

beforeAll(async () => {
  await initializeI18n(window.localStorage, ['en']);
});
afterEach(() => {
  cleanup();
  vi.unstubAllGlobals();
});

const sol: PrivateRoomAgentKnock = {
  agentId: SOL,
  displayName: 'Sol',
  expiresAtUnixMs: KNOCKED_AT + 3_600_000,
  knockedAtUnixMs: KNOCKED_AT,
};

const solInside: PrivateRoomAgentMember = {
  agentId: SOL,
  displayName: 'Sol',
  joinedAtUnixMs: KNOCKED_AT + 60_000,
  ownerDisplayName: null,
  status: 'joined',
  statusChangedAtUnixMs: KNOCKED_AT + 60_000,
};

type Knocks = Result<readonly PrivateRoomAgentKnock[], PrivateRoomFailure>;

function gateway(
  access: Result<PrivateRoomAgentAccess, PrivateRoomFailure>,
  knocks: Knocks = ok([]),
) {
  const unavailable = () => Promise.resolve(err({ code: 'test.unavailable', retryable: false }));
  const generateJoinCode = vi.fn(() =>
    Promise.resolve(ok({ code: CODE, createdAtUnixMs: Date.UTC(2026, 8, 28, 14) })),
  );
  const agentKnocks = vi.fn(() => Promise.resolve(knocks));
  const admitKnock = vi.fn<PrivateRoomGateway['admitKnock']>(() => Promise.resolve(ok(solInside)));
  const declineKnock = vi.fn<PrivateRoomGateway['declineKnock']>(() =>
    Promise.resolve(ok(undefined)),
  );
  const value: PrivateRoomGateway = {
    accept: unavailable,
    admitKnock,
    agentAccess: () => Promise.resolve(access),
    agentKnocks,
    archive: unavailable,
    ban: unavailable,
    create: unavailable,
    decline: unavailable,
    declineKnock,
    disableJoinCode: unavailable,
    generateJoinCode,
    inspect: unavailable,
    invite: unavailable,
    leave: unavailable,
    list: () => Promise.resolve(ok([])),
    remove: unavailable,
    removeCodeAgent: unavailable,
    rename: unavailable,
    transferOwnership: unavailable,
    updatePermissions: unavailable,
  };
  return { admitKnock, agentKnocks, declineKnock, generateJoinCode, value };
}

/** 口令状态查完之前按钮是灰的，等它能点再点。 */
async function clickWhenReady(name: string): Promise<void> {
  const button = await screen.findByRole('button', { name });
  await waitFor(() => {
    expect(button).toBeEnabled();
  });
  fireEvent.click(button);
}

/** 口令收在折叠里，先展开。 */
async function openCodeDetails(): Promise<void> {
  fireEvent.click(await screen.findByText('Want the agent in without waiting? Use a code'));
}

function renderInvite(rooms: PrivateRoomGateway) {
  const client = new QueryClient({ defaultOptions: { queries: { retry: false } } });
  render(
    <QueryClientProvider client={client}>
      <I18nextProvider i18n={i18n}>
        <PrivateRoomNetworkInvite
          catalogId={CATALOG}
          guide={GUIDE}
          roomName="new game dev"
          rooms={rooms}
        />
      </I18nextProvider>
    </QueryClientProvider>,
  );
}

describe('私人房间里请网络 Agent：敲门', () => {
  it('给 Agent 的话里只有房间号、没有口令，打开就能复制；没人敲门时说在等它敲门', async () => {
    const rooms = gateway(ok({ agents: [], joinCode: null }));
    renderInvite(rooms.value);

    expect(
      await screen.findByText(/a network agent knocks with the room number, and you let it in/u),
    ).toBeVisible();
    const message = screen.getByText(/knock on the Agent Room private room “new game dev”/u);
    expect(message).toHaveTextContent(CATALOG);
    expect(message).toHaveTextContent(GUIDE);
    expect(message).toHaveTextContent(/Once I let you in/u);
    expect(screen.getByRole('button', { name: 'Copy message' })).toBeVisible();
    expect(
      await screen.findByText(/When it knocks, it shows up here for you to let in/u),
    ).toBeVisible();
    expect(rooms.agentKnocks).toHaveBeenCalledWith(CATALOG);
    expect(rooms.generateJoinCode).not.toHaveBeenCalled();
    expect(screen.getByText(/the server can read what is said in this room/u)).toBeVisible();
  });

  it('有 Agent 敲门就显示出来，点“让它进来”由服务器替它进房间', async () => {
    const rooms = gateway(ok({ agents: [], joinCode: null }), ok([sol]));
    renderInvite(rooms.value);

    const list = await screen.findByRole('list', { name: 'Agents knocking' });
    expect(within(list).getByText('Sol is knocking')).toBeVisible();
    expect(screen.getByText(/can read what is sent to it from then on/u)).toBeVisible();
    rooms.agentKnocks.mockResolvedValue(ok([]));
    fireEvent.click(within(list).getByRole('button', { name: 'Let Sol in' }));

    expect(await screen.findByText('Sol is in')).toBeVisible();
    expect(rooms.admitKnock).toHaveBeenCalledWith(CATALOG, SOL);
    expect(rooms.declineKnock).not.toHaveBeenCalled();
    expect(screen.queryByRole('list', { name: 'Agents knocking' })).not.toBeInTheDocument();
  });

  it('不让进的从列表里拿掉', async () => {
    const rooms = gateway(ok({ agents: [], joinCode: null }), ok([sol]));
    renderInvite(rooms.value);

    const decline = await screen.findByRole('button', { name: 'Turn Sol away' });
    rooms.agentKnocks.mockResolvedValue(ok([]));
    fireEvent.click(decline);

    expect(await screen.findByText('Turned Sol away')).toBeVisible();
    expect(rooms.declineKnock).toHaveBeenCalledWith(CATALOG, SOL);
    expect(rooms.admitKnock).not.toHaveBeenCalled();
  });

  it('服务器这会儿没让它进去时说它还在敲门，可以再点；它已经不在敲门时说清楚原因', async () => {
    const rooms = gateway(ok({ agents: [], joinCode: null }), ok([sol]));
    rooms.admitKnock.mockResolvedValueOnce(
      err({ code: 'private_room.agent_entry_unavailable', retryable: true }),
    );
    renderInvite(rooms.value);

    fireEvent.click(await screen.findByRole('button', { name: 'Let Sol in' }));
    expect(await screen.findByText(/It’s still knocking; try again in a moment/u)).toBeVisible();
    expect(screen.getByRole('button', { name: 'Let Sol in' })).toBeEnabled();

    rooms.admitKnock.mockResolvedValueOnce(
      err({ code: 'agent_knock.not_found', retryable: false }),
    );
    fireEvent.click(screen.getByRole('button', { name: 'Let Sol in' }));
    expect(await screen.findByText(/It isn’t knocking anymore/u)).toBeVisible();
    expect(screen.queryByText('Sol is knocking')).not.toBeInTheDocument();
  });

  it('不是管理者：话里说等房间的管理者放行，不显示敲门列表，也不给口令', async () => {
    const rooms = gateway(err({ code: 'join_code.forbidden', retryable: false }));
    renderInvite(rooms.value);

    expect(
      await screen.findByText(/one of the room’s managers lets it in\./u, {
        selector: '.agent-invite__note',
      }),
    ).toBeVisible();
    expect(screen.getByText(/Once one of the room’s managers lets you in/u)).toHaveTextContent(
      CATALOG,
    );
    expect(screen.getByText(/one of the room’s managers has to let it in/u)).toBeVisible();
    expect(screen.queryByText('Want the agent in without waiting? Use a code')).toBeNull();
    expect(rooms.agentKnocks).not.toHaveBeenCalled();
    expect(rooms.generateJoinCode).not.toHaveBeenCalled();
  });
});

describe('私人房间里请网络 Agent：口令', () => {
  it('收在折叠里；一个按钮生成口令、把给 Agent 的整段话复制好，并留在对话框里', async () => {
    const writeText = vi.fn<(text: string) => Promise<void>>(() => Promise.resolve());
    vi.stubGlobal('navigator', { clipboard: { writeText } });
    const rooms = gateway(ok({ agents: [], joinCode: null }));
    renderInvite(rooms.value);

    await openCodeDetails();
    expect(screen.getByText(/comes straight in without knocking/u)).toBeVisible();
    await clickWhenReady('Create code and copy');

    expect(await screen.findByText('Copied. Send it to your agent.')).toBeVisible();
    expect(rooms.generateJoinCode).toHaveBeenCalledWith(CATALOG);
    const copied = writeText.mock.calls[0]?.[0] ?? '';
    expect(copied).toContain(CODE);
    expect(copied).toContain(GUIDE);
    expect(copied).toContain('new game dev');
    expect(screen.getByText(copied)).toBeVisible();
    expect(screen.getByText(/shown only this once/u)).toBeVisible();
  });

  it('已经有口令时说明会换一个新的，旧的随即失效', async () => {
    const rooms = gateway(ok({ agents: [], joinCode: { createdAtUnixMs: Date.UTC(2026, 8, 27) } }));
    renderInvite(rooms.value);

    await openCodeDetails();
    expect(await screen.findByRole('button', { name: 'Create a new code and copy' })).toBeVisible();
    expect(screen.getByText(/The old code stops working; agents already here stay/u)).toBeVisible();
  });

  it('自动复制不了时不报错，照样给出这段话；自己点复制也失败才提示手动复制', async () => {
    const writeText = vi.fn(() => Promise.reject(new Error('denied')));
    vi.stubGlobal('navigator', { clipboard: { writeText } });
    const rooms = gateway(ok({ agents: [], joinCode: null }));
    renderInvite(rooms.value);

    await openCodeDetails();
    await clickWhenReady('Create code and copy');

    const details = screen
      .getByText('Want the agent in without waiting? Use a code')
      .closest('details');
    if (details === null) throw new Error('口令应该收在折叠里');
    const copyButton = await within(details).findByRole('button', {
      name: 'Copy message for the agent',
    });
    expect(within(details).getByText(new RegExp(CODE, 'u'))).toBeVisible();
    expect(screen.queryByText(/Couldn’t copy/u)).not.toBeInTheDocument();
    expect(screen.queryByText('Copied. Send it to your agent.')).not.toBeInTheDocument();
    fireEvent.click(copyButton);
    expect(await screen.findByText(/Couldn’t copy. Select the text above/u)).toBeVisible();
    expect(writeText).toHaveBeenCalledTimes(2);
  });
});
