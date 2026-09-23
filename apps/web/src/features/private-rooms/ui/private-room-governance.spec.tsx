// @vitest-environment jsdom
import '@testing-library/jest-dom/vitest';
import { QueryClient, QueryClientProvider } from '@tanstack/react-query';
import { cleanup, fireEvent, render, screen, waitFor } from '@testing-library/react';
import { I18nextProvider } from 'react-i18next';
import { afterEach, beforeAll, describe, expect, it, vi } from 'vitest';

import type { PrivateRoomCoordinator } from '@/features/private-rooms/application/private-room-coordinator';
import type {
  PrivateRoom,
  PrivateRoomAgentAccess,
  PrivateRoomGateway,
} from '@/features/private-rooms/domain/private-room';
import { i18n, initializeI18n } from '@/shared/i18n/i18n';
import { err, ok } from '@/shared/result';

import { PrivateRoomGovernance } from './private-room-governance';

const OWNER = '0198b601-77a1-7bb8-83eb-a8fe68c97e42';
const MEMBER = '0198b601-77a1-7bb8-83eb-a8fe68c97e43';

const room: PrivateRoom = {
  catalogId: '0198b601-77a1-7bb8-83eb-a8fe68c97e46',
  description: 'Private review',
  matrixRoomId: '!private:matrix.test',
  members: [
    {
      permissions: { capabilities: ['view', 'speak', 'invite', 'manage', 'automate'] },
      principalId: OWNER,
      status: 'joined',
    },
    {
      permissions: { capabilities: ['view', 'speak'] },
      principalId: MEMBER,
      status: 'joined',
    },
  ],
  name: 'Architecture room',
  ownerPrincipalId: OWNER,
  retentionDays: 30,
  roomInstanceId: '0198b601-77a1-7bb8-83eb-a8fe68c97e47',
  status: 'active',
  version: 2,
};

const AGENT = '0198b601-77a1-7bb8-83eb-a8fe68c97e48';
const CREATED = Date.UTC(2026, 8, 23, 6, 0);

/** 和服务器一样记着口令状态：生成、停用、移出之后再查看拿到的是新状态。 */
function gateway(initial: PrivateRoomAgentAccess = { agents: [], joinCode: null }) {
  const unavailable = () => Promise.resolve(err({ code: 'test.unavailable', retryable: false }));
  const rename = vi.fn((_catalogId: string, name: string) =>
    Promise.resolve(ok({ ...room, name })),
  );
  let access = initial;
  const agentAccess = vi.fn(() => Promise.resolve(ok(access)));
  const generateJoinCode = vi.fn(() => {
    const createdAtUnixMs = CREATED + 60_000;
    access = { ...access, joinCode: { createdAtUnixMs } };
    return Promise.resolve(ok({ code: 'K7P3-Q9XW-2DMA', createdAtUnixMs }));
  });
  const disableJoinCode = vi.fn(() => {
    access = { ...access, joinCode: null };
    return Promise.resolve(ok(undefined));
  });
  const removeCodeAgent = vi.fn((_catalogId: string, agentId: string) => {
    access = {
      ...access,
      agents: access.agents.map((agent) =>
        agent.agentId === agentId ? { ...agent, status: 'removed' } : agent,
      ),
    };
    return Promise.resolve(ok(undefined));
  });
  const value: PrivateRoomGateway = {
    accept: unavailable,
    agentAccess,
    archive: unavailable,
    ban: unavailable,
    create: unavailable,
    decline: unavailable,
    disableJoinCode,
    generateJoinCode,
    inspect: unavailable,
    invite: unavailable,
    leave: unavailable,
    list: () => Promise.resolve(ok([])),
    remove: unavailable,
    removeCodeAgent,
    rename,
    transferOwnership: unavailable,
    updatePermissions: unavailable,
  };
  return { agentAccess, disableJoinCode, generateJoinCode, removeCodeAgent, rename, value };
}

function renderGovernance(principalId: string, rooms: PrivateRoomGateway) {
  return render(
    <I18nextProvider i18n={i18n}>
      <QueryClientProvider client={new QueryClient()}>
        <PrivateRoomGovernance
          coordinator={{} as PrivateRoomCoordinator}
          onExitRoom={() => undefined}
          principalId={principalId}
          recentlyAuthenticated={false}
          room={room}
          rooms={rooms}
        />
      </QueryClientProvider>
    </I18nextProvider>,
  );
}

beforeAll(async () => {
  await initializeI18n(window.localStorage, ['en-US']);
});

afterEach(() => {
  cleanup();
  vi.unstubAllGlobals();
});

describe('PrivateRoomGovernance', () => {
  it('房主可以改名：名字没变或为空时不能提交', async () => {
    const rooms = gateway();
    renderGovernance(OWNER, rooms.value);
    const input = screen.getByLabelText('Room name');
    const button = screen.getByRole('button', { name: 'Rename' });
    expect(input).toHaveValue('Architecture room');
    expect(button).toBeDisabled();
    fireEvent.change(input, { target: { value: '   ' } });
    expect(button).toBeDisabled();
    fireEvent.change(input, { target: { value: '  Design review ' } });
    expect(button).toBeEnabled();
    fireEvent.click(button);
    await waitFor(() => {
      expect(rooms.rename).toHaveBeenCalledWith(room.catalogId, 'Design review');
    });
  });

  it('成员只看到房间名，不能改', () => {
    const rooms = gateway();
    renderGovernance(MEMBER, rooms.value);
    expect(screen.getByText('Architecture room')).toBeVisible();
    expect(screen.queryByLabelText('Room name')).toBeNull();
    expect(screen.queryByRole('button', { name: 'Rename' })).toBeNull();
  });

  it('房主生成口令后看到口令，一键复制给 Agent 的话', async () => {
    const writeText = vi.fn<(text: string) => Promise<void>>(() => Promise.resolve());
    vi.stubGlobal('navigator', { clipboard: { writeText } });
    const rooms = gateway();
    renderGovernance(OWNER, rooms.value);
    expect(await screen.findByText(/No code yet/u)).toBeVisible();
    fireEvent.click(screen.getByRole('button', { name: 'Create code' }));
    expect(await screen.findByText('K7P3-Q9XW-2DMA')).toBeVisible();
    expect(rooms.generateJoinCode).toHaveBeenCalledWith(room.catalogId);
    expect(screen.getByText(/Shown only this once/u)).toBeVisible();
    fireEvent.click(screen.getByRole('button', { name: 'Copy message for the agent' }));
    expect(await screen.findByRole('button', { name: 'Copied' })).toBeVisible();
    const message = writeText.mock.calls[0]?.[0] ?? '';
    expect(message).toContain('agent-room join --code K7P3-Q9XW-2DMA');
    expect(message).toContain('"Architecture room"');
    expect(message).toContain('agent_room_join');
    // 口令开着时只能换或停用，不再显示生成按钮。
    expect(screen.getByRole('button', { name: 'Replace code' })).toBeVisible();
    expect(screen.queryByRole('button', { name: 'Create code' })).toBeNull();
  });

  it('复制失败时展开给 Agent 的话，让人手动复制', async () => {
    const writeText = vi.fn<(text: string) => Promise<void>>(() =>
      Promise.reject(new Error('denied')),
    );
    vi.stubGlobal('navigator', { clipboard: { writeText } });
    const rooms = gateway();
    renderGovernance(OWNER, rooms.value);
    fireEvent.click(await screen.findByRole('button', { name: 'Create code' }));
    fireEvent.click(await screen.findByRole('button', { name: 'Copy message for the agent' }));
    expect(await screen.findByText(/Copy failed/u)).toBeVisible();
    const preview = screen.getByText(/agent-room join --code K7P3-Q9XW-2DMA/u);
    expect(preview).toBeVisible();
    expect(preview.closest('details')).toHaveAttribute('open');
  });

  it('已有口令只显示创建时间；停用后回到生成', async () => {
    const rooms = gateway({ agents: [], joinCode: { createdAtUnixMs: CREATED } });
    renderGovernance(OWNER, rooms.value);
    expect(await screen.findByText(/Code on since/u)).toBeVisible();
    expect(screen.queryByText('K7P3-Q9XW-2DMA')).toBeNull();
    fireEvent.click(screen.getByRole('button', { name: 'Turn off code' }));
    await waitFor(() => {
      expect(rooms.disableJoinCode).toHaveBeenCalledWith(room.catalogId);
    });
    expect(await screen.findByRole('button', { name: 'Create code' })).toBeVisible();
  });

  it('列出凭口令进来的 Agent 与主人，移除后不再列出', async () => {
    const rooms = gateway({
      agents: [
        {
          agentId: AGENT,
          displayName: 'Scout',
          joinedAtUnixMs: CREATED,
          ownerDisplayName: 'Mina',
          status: 'joined',
          statusChangedAtUnixMs: CREATED,
        },
        {
          agentId: '0198b601-77a1-7bb8-83eb-a8fe68c97e49',
          displayName: 'Gone',
          joinedAtUnixMs: CREATED,
          ownerDisplayName: null,
          status: 'removed',
          statusChangedAtUnixMs: CREATED + 1,
        },
      ],
      joinCode: { createdAtUnixMs: CREATED },
    });
    renderGovernance(OWNER, rooms.value);
    const list = await screen.findByRole('list', { name: 'Agents that joined with the code' });
    expect(list).toHaveTextContent('Scout');
    expect(list).toHaveTextContent('Agent of Mina');
    expect(list).not.toHaveTextContent('Gone');
    fireEvent.click(screen.getByRole('button', { name: 'Remove Scout' }));
    await waitFor(() => {
      expect(rooms.removeCodeAgent).toHaveBeenCalledWith(room.catalogId, AGENT);
    });
    expect(await screen.findByText('No agent has joined with the code yet.')).toBeVisible();
  });

  it('不能管理房间的成员看不到口令，也不去查', () => {
    const rooms = gateway();
    renderGovernance(MEMBER, rooms.value);
    expect(screen.queryByRole('heading', { name: 'Agent code' })).toBeNull();
    expect(rooms.agentAccess).not.toHaveBeenCalled();
  });
});
