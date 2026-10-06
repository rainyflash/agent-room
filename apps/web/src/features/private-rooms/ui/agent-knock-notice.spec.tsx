// @vitest-environment jsdom
import '@testing-library/jest-dom/vitest';
import { QueryClient, QueryClientProvider } from '@tanstack/react-query';
import { act, cleanup, render, screen, waitFor } from '@testing-library/react';
import { I18nextProvider } from 'react-i18next';
import { afterEach, beforeAll, describe, expect, it, vi } from 'vitest';

import type { PrivateRoom, PrivateRoomGateway } from '@/features/private-rooms/domain/private-room';
import { i18n, initializeI18n } from '@/shared/i18n/i18n';
import { err, ok } from '@/shared/result';

import { AgentKnockNotice, lobbyCatalogId } from './agent-knock-notice';

const OWNER = '0198b601-77a1-7bb8-83eb-a8fe68c97e42';
const MEMBER = '0198b601-77a1-7bb8-83eb-a8fe68c97e43';
const CATALOG = '0198b601-77a1-7bb8-83eb-a8fe68c97e46';
const KNOCKED_AT = Date.UTC(2026, 9, 6, 9);

// 被测组件从这两个钩子拿服务和当前账户；每个用例渲染前换掉。
const services = vi.hoisted(() => {
  const state: { privateRooms: unknown; principalId: string } = {
    privateRooms: null,
    principalId: '',
  };
  return state;
});
vi.mock('@/app/app-services', () => ({
  useAppServices: () => ({ privateRooms: services.privateRooms }),
}));
vi.mock('@/features/session/ui/session-provider', () => ({
  useSession: () => ({
    snapshot: { context: { principal: { principalId: services.principalId } } },
  }),
}));

beforeAll(async () => {
  await initializeI18n(window.localStorage, ['en']);
});
afterEach(() => {
  cleanup();
});

const room: PrivateRoom = {
  catalogId: CATALOG,
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

function gateway() {
  const unavailable = () => Promise.resolve(err({ code: 'test.unavailable', retryable: false }));
  const agentKnocks = vi.fn<PrivateRoomGateway['agentKnocks']>(() =>
    Promise.resolve(
      ok([
        {
          agentId: '0198b601-77a1-7bb8-83eb-a8fe68c97e51',
          displayName: 'Sol',
          expiresAtUnixMs: KNOCKED_AT + 3_600_000,
          knockedAtUnixMs: KNOCKED_AT,
        },
      ]),
    ),
  );
  const list = vi.fn(() => Promise.resolve(ok([room])));
  const value: PrivateRoomGateway = {
    accept: unavailable,
    admitKnock: unavailable,
    agentAccess: unavailable,
    agentKnocks,
    archive: unavailable,
    ban: unavailable,
    create: unavailable,
    decline: unavailable,
    declineKnock: unavailable,
    disableJoinCode: unavailable,
    generateJoinCode: unavailable,
    inspect: unavailable,
    invite: unavailable,
    leave: unavailable,
    list,
    remove: unavailable,
    removeCodeAgent: unavailable,
    rename: unavailable,
    transferOwnership: unavailable,
    updatePermissions: unavailable,
  };
  return { agentKnocks, list, value };
}

function renderNotice(pathname: string, principalId: string) {
  const rooms = gateway();
  services.privateRooms = rooms.value;
  services.principalId = principalId;
  const client = new QueryClient({ defaultOptions: { queries: { retry: false } } });
  render(
    <QueryClientProvider client={client}>
      <I18nextProvider i18n={i18n}>
        <AgentKnockNotice pathname={pathname} />
      </I18nextProvider>
    </QueryClientProvider>,
  );
  return rooms;
}

describe('提示栈里的敲门', () => {
  it('只认房间页 /lobby/<房间号>', () => {
    expect(lobbyCatalogId(`/lobby/${CATALOG}`)).toBe(CATALOG);
    expect(lobbyCatalogId(`/lobby/${CATALOG}/`)).toBe(CATALOG);
    expect(lobbyCatalogId(`/lobby/${CATALOG}/instance/!room:matrix.test`)).toBeNull();
    expect(lobbyCatalogId('/rooms')).toBeNull();
    expect(lobbyCatalogId('/settings/general')).toBeNull();
  });

  it('管理者开着私人房间时，有 Agent 敲门就挂一条', async () => {
    renderNotice(`/lobby/${CATALOG}`, OWNER);

    expect(await screen.findByText('Sol is knocking')).toBeVisible();
    expect(screen.getByRole('button', { name: 'Let Sol in' })).toBeVisible();
  });

  it('不是管理者不问有没有人敲门', async () => {
    const rooms = renderNotice(`/lobby/${CATALOG}`, MEMBER);

    await waitFor(() => {
      expect(rooms.list).toHaveBeenCalled();
    });
    await act(async () => {
      await Promise.resolve();
    });
    expect(rooms.agentKnocks).not.toHaveBeenCalled();
    expect(screen.queryByText('Sol is knocking')).not.toBeInTheDocument();
  });

  it('不在房间页时连私人房间列表都不读', () => {
    const rooms = renderNotice('/rooms', OWNER);

    expect(rooms.list).not.toHaveBeenCalled();
    expect(rooms.agentKnocks).not.toHaveBeenCalled();
  });
});
