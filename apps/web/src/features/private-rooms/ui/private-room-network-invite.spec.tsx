// @vitest-environment jsdom
import '@testing-library/jest-dom/vitest';
import { QueryClient, QueryClientProvider } from '@tanstack/react-query';
import { cleanup, fireEvent, render, screen, waitFor } from '@testing-library/react';
import { I18nextProvider } from 'react-i18next';
import { afterEach, beforeAll, describe, expect, it, vi } from 'vitest';

import type {
  PrivateRoomAgentAccess,
  PrivateRoomFailure,
  PrivateRoomGateway,
} from '@/features/private-rooms/domain/private-room';
import { i18n, initializeI18n } from '@/shared/i18n/i18n';
import { err, ok, type Result } from '@/shared/result';

import { PrivateRoomNetworkInvite } from './private-room-network-invite';

const CATALOG = '0198b601-77a1-7bb8-83eb-a8fe68c97e46';
const GUIDE = 'https://app.agent-room.test/agents.md';
const CODE = 'K7P3-Q9XW-2DMA';

beforeAll(async () => {
  await initializeI18n(window.localStorage, ['en']);
});
afterEach(() => {
  cleanup();
  vi.unstubAllGlobals();
});

function gateway(access: Result<PrivateRoomAgentAccess, PrivateRoomFailure>) {
  const unavailable = () => Promise.resolve(err({ code: 'test.unavailable', retryable: false }));
  const generateJoinCode = vi.fn(() =>
    Promise.resolve(ok({ code: CODE, createdAtUnixMs: Date.UTC(2026, 8, 28, 14) })),
  );
  const value: PrivateRoomGateway = {
    accept: unavailable,
    agentAccess: () => Promise.resolve(access),
    archive: unavailable,
    ban: unavailable,
    create: unavailable,
    decline: unavailable,
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
  return { generateJoinCode, value };
}

/** 口令状态查完之前按钮是灰的，等它能点再点。 */
async function clickWhenReady(name: string): Promise<void> {
  const button = await screen.findByRole('button', { name });
  await waitFor(() => {
    expect(button).toBeEnabled();
  });
  fireEvent.click(button);
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

describe('私人房间里请网络 Agent', () => {
  it('一个按钮生成口令、把给 Agent 的整段话复制好，并留在对话框里', async () => {
    const writeText = vi.fn<(text: string) => Promise<void>>(() => Promise.resolve());
    vi.stubGlobal('navigator', { clipboard: { writeText } });
    const rooms = gateway(ok({ agents: [], joinCode: null }));
    renderInvite(rooms.value);

    expect(
      screen.getByText(/is a private room, so a network agent needs this room’s code/u),
    ).toBeVisible();
    expect(screen.getByText(/The button below creates the code/u)).toBeVisible();
    await clickWhenReady('Create code and copy');

    expect(await screen.findByText('Copied. Send it to your agent.')).toBeVisible();
    expect(rooms.generateJoinCode).toHaveBeenCalledWith(CATALOG);
    const copied = writeText.mock.calls[0]?.[0] ?? '';
    expect(copied).toContain(CODE);
    expect(copied).toContain(GUIDE);
    expect(copied).toContain('new game dev');
    expect(screen.getByText(copied)).toBeVisible();
    expect(screen.getByText(/shown only this once/u)).toBeVisible();
    expect(screen.getByText(/the server can read what is said in this room/u)).toBeVisible();
  });

  it('已经有口令时说明会换一个新的，旧的随即失效', async () => {
    const rooms = gateway(ok({ agents: [], joinCode: { createdAtUnixMs: Date.UTC(2026, 8, 27) } }));
    renderInvite(rooms.value);

    expect(await screen.findByRole('button', { name: 'Create a new code and copy' })).toBeVisible();
    expect(screen.getByText(/The old code stops working; agents already here stay/u)).toBeVisible();
  });

  it('不是管理者时直接说明，不给按钮', async () => {
    const rooms = gateway(err({ code: 'join_code.forbidden', retryable: false }));
    renderInvite(rooms.value);

    expect(await screen.findByText(/Only this room’s managers can create a code/u)).toBeVisible();
    expect(screen.queryByRole('button')).not.toBeInTheDocument();
    expect(screen.queryByText(/The button below/u)).not.toBeInTheDocument();
    expect(rooms.generateJoinCode).not.toHaveBeenCalled();
  });

  it('自动复制不了时不报错，照样给出这段话；自己点复制也失败才提示手动复制', async () => {
    const writeText = vi.fn(() => Promise.reject(new Error('denied')));
    vi.stubGlobal('navigator', { clipboard: { writeText } });
    const rooms = gateway(ok({ agents: [], joinCode: null }));
    renderInvite(rooms.value);

    await clickWhenReady('Create code and copy');

    const copyButton = await screen.findByRole('button', { name: 'Copy message for the agent' });
    expect(screen.getByText(new RegExp(CODE, 'u'))).toBeVisible();
    expect(screen.queryByText(/Copy failed/u)).not.toBeInTheDocument();
    fireEvent.click(copyButton);
    expect(await screen.findByText(/Copy failed. Select the message above/u)).toBeVisible();
    expect(writeText).toHaveBeenCalledTimes(2);
  });
});
