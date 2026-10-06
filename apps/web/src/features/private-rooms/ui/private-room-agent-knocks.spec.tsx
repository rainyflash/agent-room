// @vitest-environment jsdom
import '@testing-library/jest-dom/vitest';
import { QueryClient, QueryClientProvider } from '@tanstack/react-query';
import { act, cleanup, fireEvent, render, screen, waitFor } from '@testing-library/react';
import type { ReactNode } from 'react';
import { I18nextProvider } from 'react-i18next';
import { afterEach, beforeAll, describe, expect, it, vi } from 'vitest';

import type {
  PrivateRoomAgentKnock,
  PrivateRoomGateway,
} from '@/features/private-rooms/domain/private-room';
import { i18n, initializeI18n } from '@/shared/i18n/i18n';
import { err, ok } from '@/shared/result';

import { PrivateRoomAgentKnocks, PrivateRoomAgentKnockToasts } from './private-room-agent-knocks';

const CATALOG = '0198b601-77a1-7bb8-83eb-a8fe68c97e46';
const KNOCKED_AT = Date.UTC(2026, 9, 6, 9);

beforeAll(async () => {
  await initializeI18n(window.localStorage, ['en']);
});
afterEach(() => {
  cleanup();
});

function knock(index: number, displayName: string): PrivateRoomAgentKnock {
  return {
    agentId: `0198b601-77a1-7bb8-83eb-a8fe68c97e5${String(index)}`,
    displayName,
    expiresAtUnixMs: KNOCKED_AT + 3_600_000,
    knockedAtUnixMs: KNOCKED_AT + index,
  };
}

function gateway(knocks: readonly PrivateRoomAgentKnock[] | 'forbidden') {
  const unavailable = () => Promise.resolve(err({ code: 'test.unavailable', retryable: false }));
  const admitKnock = vi.fn<PrivateRoomGateway['admitKnock']>(unavailable);
  const agentKnocks = vi.fn<PrivateRoomGateway['agentKnocks']>(() =>
    Promise.resolve(
      knocks === 'forbidden'
        ? err({ code: 'agent_knock.forbidden', retryable: false })
        : ok(knocks),
    ),
  );
  return {
    admitKnock,
    agentKnocks,
    value: {
      accept: unavailable,
      admitKnock,
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
      list: () => Promise.resolve(ok([])),
      remove: unavailable,
      removeCodeAgent: unavailable,
      rename: unavailable,
      transferOwnership: unavailable,
      updatePermissions: unavailable,
    } satisfies PrivateRoomGateway,
  };
}

function renderWithQueries(children: ReactNode) {
  const client = new QueryClient({ defaultOptions: { queries: { retry: false } } });
  render(
    <QueryClientProvider client={client}>
      <I18nextProvider i18n={i18n}>{children}</I18nextProvider>
    </QueryClientProvider>,
  );
}

describe('房间页提示栈里的敲门', () => {
  it('同时最多两条：多的合成一条“还有 N 个在敲门”', async () => {
    const rooms = gateway([knock(1, 'Sol'), knock(2, 'Atlas'), knock(3, 'Vega')]);
    renderWithQueries(<PrivateRoomAgentKnockToasts catalogId={CATALOG} rooms={rooms.value} />);

    expect(await screen.findByText('Sol is knocking')).toBeVisible();
    expect(screen.getByText('2 more agents are knocking')).toBeVisible();
    expect(screen.queryByText('Atlas is knocking')).not.toBeInTheDocument();
    expect(screen.getByRole('button', { name: 'Let Sol in' })).toBeVisible();
  });

  it('两个以内各挂一条，每条都能直接放行', async () => {
    const rooms = gateway([knock(1, 'Sol'), knock(2, 'Atlas')]);
    renderWithQueries(<PrivateRoomAgentKnockToasts catalogId={CATALOG} rooms={rooms.value} />);

    expect(await screen.findByText('Atlas is knocking')).toBeVisible();
    expect(screen.getByText('Sol is knocking')).toBeVisible();
    expect(screen.queryByText(/more agents? (is|are) knocking/u)).not.toBeInTheDocument();
    fireEvent.click(screen.getByRole('button', { name: 'Let Atlas in' }));
    await waitFor(() => {
      expect(rooms.admitKnock).toHaveBeenCalledWith(CATALOG, knock(2, 'Atlas').agentId);
    });
  });

  it('读不到敲门（不是管理者、旧版服务器）时什么都不显示', async () => {
    const rooms = gateway('forbidden');
    renderWithQueries(
      <>
        <PrivateRoomAgentKnockToasts catalogId={CATALOG} rooms={rooms.value} />
        <PrivateRoomAgentKnocks catalogId={CATALOG} emptyText="Nobody yet" rooms={rooms.value} />
      </>,
    );

    await waitFor(() => {
      expect(rooms.agentKnocks).toHaveBeenCalledTimes(1);
    });
    await act(async () => {
      await Promise.resolve();
    });
    expect(screen.queryByText('Nobody yet')).not.toBeInTheDocument();
    expect(screen.queryByRole('status')).not.toBeInTheDocument();
  });
});
