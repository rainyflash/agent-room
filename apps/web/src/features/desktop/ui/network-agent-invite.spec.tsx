// @vitest-environment jsdom

import '@testing-library/jest-dom/vitest';

import { cleanup, fireEvent, render, screen } from '@testing-library/react';
import { I18nextProvider } from 'react-i18next';
import { afterEach, beforeAll, describe, expect, it, vi } from 'vitest';

import { useOptionalAppServices } from '@/app/app-services';
import type { PublicRoomSummary } from '@/features/room-directory/domain/public-room-directory';
import { i18n, initializeI18n } from '@/shared/i18n/i18n';
import { NetworkAgentInvite, networkInviteTarget } from './network-agent-invite';

vi.mock('@/app/app-services', () => ({ useOptionalAppServices: vi.fn() }));

beforeAll(async () => {
  await initializeI18n(window.localStorage, ['en']);
});

afterEach(() => {
  cleanup();
  vi.clearAllMocks();
});

const lobby: PublicRoomSummary = {
  catalogId: '0198b601-77a3-74f1-b4f4-940f291951b1',
  slug: 'agent-room-global',
  name: 'Agent Room Global',
  description: '',
  language: null,
  activeInstanceCount: 1,
  onlineAgentCount: 0,
};

describe('只凭网络接入能进哪里', () => {
  it('当前房间在公开大厅目录里就进这一间', () => {
    expect(networkInviteTarget(lobby.catalogId, [lobby])).toEqual({
      kind: 'lobby',
      name: 'Agent Room Global',
    });
  });

  it('目录里没有当前房间就是私人房间，网络 Agent 要凭口令进', () => {
    expect(networkInviteTarget('0198b601-77a3-74f1-b4f4-940f291951b2', [lobby])).toEqual({
      kind: 'private',
    });
  });

  it('没有房间上下文或还不知道目录时只说进公开大厅', () => {
    expect(networkInviteTarget(undefined, [lobby])).toEqual({ kind: 'lobby', name: null });
    expect(networkInviteTarget(lobby.catalogId, null)).toEqual({ kind: 'lobby', name: null });
  });
});

describe('网页里只能聊天的 Agent', () => {
  function services(desktop: boolean) {
    return {
      config: { controlPlaneUrl: 'https://api.agent-room.test' },
      localRuntime: { isAvailable: () => desktop },
      roomDirectory: { list: vi.fn() },
    } as unknown as ReturnType<typeof useOptionalAppServices>;
  }

  function renderInvite() {
    return render(
      <I18nextProvider i18n={i18n}>
        <NetworkAgentInvite room={null} />
      </I18nextProvider>,
    );
  }

  it('桌面端：说清要靠它所在的应用加 MCP 连接器，地址直接给出来', () => {
    vi.mocked(useOptionalAppServices).mockReturnValue(services(true));
    renderInvite();

    // 给 Agent 的那段话也说了读不了网页、发不了请求时怎么办，不点名任何应用。
    expect(screen.getByText(/tell me: if your app lets me add an MCP connector/u)).toBeVisible();
    expect(screen.getByText(/can only come in through its app/u)).not.toBeVisible();
    fireEvent.click(screen.getByText('Agent in a chat web page?'));
    expect(screen.getByText(/can only come in through its app/u)).toBeVisible();
    expect(screen.getByText(/this kind of agent can’t come in/u)).toBeVisible();
    expect(screen.getByText('https://api.agent-room.test/mcp')).toBeVisible();
    expect(screen.getByRole('button', { name: 'Copy address' })).toBeVisible();
  });

  it('网页端：不知道服务器的 API 域名，让人去说明开头找地址', () => {
    vi.mocked(useOptionalAppServices).mockReturnValue(services(false));
    renderInvite();

    fireEvent.click(screen.getByText('Agent in a chat web page?'));
    expect(screen.getByText(/The address is at the top of http.*\/agents\.txt/u)).toBeVisible();
    expect(screen.queryByRole('button', { name: 'Copy address' })).not.toBeInTheDocument();
  });
});
