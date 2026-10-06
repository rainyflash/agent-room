// @vitest-environment jsdom
import '@testing-library/jest-dom/vitest';
import { cleanup, fireEvent, render, screen, waitFor } from '@testing-library/react';
import { I18nextProvider } from 'react-i18next';
import { afterEach, beforeAll, beforeEach, describe, expect, it, vi } from 'vitest';

import { AgentInviteDialog } from './agent-invite-dialog';
import { DesktopRuntimeProvider } from './desktop-runtime-provider';
import type {
  BridgePhase,
  BridgeRuntime,
  DesktopRuntimeGateway,
  HostSessionDiagnostics,
  InvitationOffer,
} from '../domain/desktop-runtime';
import { i18n, initializeI18n } from '@/shared/i18n/i18n';
import { err, ok, type Result } from '@/shared/result';

const router = vi.hoisted(() => ({ navigate: vi.fn() }));
vi.mock('@tanstack/react-router', async (loadOriginal) => {
  const original = await loadOriginal<typeof import('@tanstack/react-router')>();
  return { ...original, useNavigate: () => router.navigate };
});

const owner = { principalId: '0198b601-77a3-74f1-b4f4-940f291951b9', displayName: 'Ada' };
const CATALOG = '0198b601-77a3-74f1-b4f4-940f291951b1';
const room = { roomId: '!builders:matrix.test', roomName: 'Builders Exchange', catalogId: CATALOG };
const appNames = /Codex|Claude Code|Cursor|CODEX_/u;

function clipboardMock() {
  return vi.fn<(text: string) => Promise<void>>(() => Promise.resolve());
}

function bridge(phase: BridgePhase): BridgeRuntime {
  return {
    authorization:
      phase === 'authorization_required'
        ? {
            promptId: '0198b601-77a3-74f1-b4f4-940f291951c9',
            verificationHost: 'identity.test',
            userCode: 'ABCD-EFGH',
            expiresAtUnixMs: Date.now() + 600_000,
          }
        : null,
    deviceReauthorizationAvailable: false,
    session: null,
    lifecycle: {
      automaticRestartCount: 0,
      changedAtUnixMs: 1,
      diagnosticCode: null,
      lastFailureCode: null,
      lastExitCode: null,
      nextRetryAtUnixMs: null,
      ownership: 'managed',
      phase,
    },
  };
}

type SessionsResult = Result<
  readonly HostSessionDiagnostics[],
  { code: string; retryable: boolean }
>;

function gateway(
  options: {
    readonly available?: boolean;
    readonly phase?: BridgePhase;
    readonly sessions?: () => SessionsResult;
  } = {},
) {
  const unavailable = () => Promise.resolve(err({ code: 'test.unavailable', retryable: false }));
  const offerInvitation = vi.fn((invitation: InvitationOffer) =>
    Promise.resolve(ok({ invitation, expiresInMs: 600_000 })),
  );
  const withdrawInvitation = vi.fn<
    (sessionKey: string) => Promise<Result<void, { code: string; retryable: boolean }>>
  >(() => Promise.resolve(ok(undefined)));
  const runtime = bridge(options.phase ?? 'ready');
  const value: DesktopRuntimeGateway = {
    beginHumanAuthentication: unavailable,
    beginMatrixAuthentication: unavailable,
    bootstrapDefaultAgent: unavailable,
    checkUpdate: unavailable,
    clearHumanSession: () => Promise.resolve(ok(undefined)),
    restoreHumanSession: () => Promise.resolve(ok(true)),
    sendControlPlaneRequest: () =>
      Promise.resolve(ok({ status: 204, headers: [], body: new Uint8Array() })),
    configureAgentRuntime: (target) => Promise.resolve(ok(target)),
    installUpdate: unavailable,
    isAvailable: () => options.available ?? true,
    openAuthorization: unavailable,
    retryBridge: () => Promise.resolve(ok(runtime)),
    reauthorizeBridge: () => Promise.resolve(ok(runtime)),
    setAutostart: (enabled) => Promise.resolve(ok(enabled)),
    snapshot: () =>
      Promise.resolve(
        ok({
          agentTarget: null,
          autostartEnabled: false,
          bridge: runtime,
          deepLink: null,
          cliConfiguration: { command: 'C:\\Agent Room\\agent-room.exe', args: [] },
          manualHostConfiguration: {
            args: [],
            command: 'C:\\Agent Room\\agent-room-mcp.exe',
            serverName: 'agent_room',
            transport: 'stdio',
          },
          platform: 'windows',
          updatesConfigured: false,
        }),
      ),
    subscribe: () => Promise.resolve(ok(() => undefined)),
    readHostSessions: () => Promise.resolve(options.sessions?.() ?? ok([])),
    offerInvitation,
    withdrawInvitation,
  };
  return { value, offerInvitation, withdrawInvitation };
}

function session(
  sessionId: string,
  state: HostSessionDiagnostics['session']['state'],
  extra: Partial<HostSessionDiagnostics> = {},
): HostSessionDiagnostics {
  return {
    displayName: 'Scout',
    roomId: state === 'starting' ? null : room.roomId,
    requestedRoom: { catalogId: CATALOG, roomId: room.roomId },
    session: {
      sessionId,
      state,
      agentId: '0198b601-77a3-74f1-b4f4-940f291951e1',
      errorCode: state === 'failed' ? 'bridge.agent_runtime_unavailable' : null,
    },
    lastInboxReadAgoMs: state === 'ready' ? 1_000 : null,
    lastMessageReceivedAgoMs: null,
    lastMessageSentAgoMs: null,
    ...extra,
  };
}

function renderDialog(
  runtime: DesktopRuntimeGateway,
  props: Partial<Parameters<typeof AgentInviteDialog>[0]> = {},
) {
  const onClose = vi.fn();
  const element = (next: Partial<Parameters<typeof AgentInviteDialog>[0]>) => (
    <I18nextProvider i18n={i18n}>
      <DesktopRuntimeProvider gateway={runtime}>
        <AgentInviteDialog
          downloadUrl="https://download.test/agent-room.exe"
          onClose={onClose}
          owner={owner}
          room={room}
          {...next}
        />
      </DesktopRuntimeProvider>
    </I18nextProvider>
  );
  const view = render(element(props));
  return {
    ...view,
    onClose,
    rerenderWith: (next: Partial<Parameters<typeof AgentInviteDialog>[0]>) => {
      view.rerender(element({ ...props, ...next }));
    },
  };
}

function chooseMethod(name: 'Network' | 'MCP' | 'Command line') {
  fireEvent.click(screen.getByRole('radio', { name }));
}
/** 接入方式存在本机；本机方式的用例从命令行或 MCP 开始。 */
function rememberMethod(method: 'network' | 'mcp' | 'cli') {
  window.localStorage.setItem('agent-room.invite-method', method);
}
async function copyMessage(writeText: ReturnType<typeof clipboardMock>): Promise<string> {
  const button = await screen.findByRole('button', { name: 'Copy message' });
  await waitFor(() => {
    expect(button).toBeEnabled();
  });
  fireEvent.click(button);
  await waitFor(() => {
    expect(writeText).toHaveBeenCalled();
  });
  return writeText.mock.calls.at(-1)?.[0] ?? '';
}

beforeAll(async () => {
  await initializeI18n(window.localStorage, ['en-US']);
});
beforeEach(() => {
  window.localStorage.clear();
});
afterEach(() => {
  cleanup();
  vi.unstubAllGlobals();
  vi.useRealTimers();
});

describe('接入 Agent 对话框', () => {
  it('一屏：默认网络方式，一段话一个复制按钮，没有编号步骤、名字和人物', async () => {
    const writeText = clipboardMock();
    vi.stubGlobal('navigator', { clipboard: { writeText } });
    renderDialog(gateway({ available: false }).value);

    expect(screen.getByRole('dialog', { name: 'Bring an agent' })).toBeVisible();
    expect(screen.getByText('It joins “Builders Exchange”.')).toBeVisible();
    expect(screen.getByRole('radio', { name: 'Network' })).toHaveAttribute('aria-checked', 'true');
    expect(screen.queryByRole('list', { name: /step/iu })).toBeNull();
    expect(screen.queryByLabelText('Agent name')).toBeNull();
    expect(screen.queryByText(/Saved characters|Invite another agent/u)).toBeNull();

    const message = await copyMessage(writeText);
    expect(message).toContain('/agents.txt');
    expect(message).toContain('untrusted input');
    expect(message).not.toMatch(appNames);
    expect(await screen.findByText('Copied. Send it to your agent.')).toBeVisible();
    expect(screen.getByText('Waiting for your agent to join…')).toBeVisible();
  });

  it('网页上的命令行：按房间名接入的一条命令，说清去哪找 CLI，给下载，不提本机面板', async () => {
    const writeText = clipboardMock();
    vi.stubGlobal('navigator', { clipboard: { writeText } });
    renderDialog(gateway({ available: false }).value);
    chooseMethod('Command line');

    expect(screen.getByRole('link', { name: 'Download Agent Room' })).toHaveAttribute(
      'href',
      'https://download.test/agent-room.exe',
    );
    const message = await copyMessage(writeText);
    expect(message).toContain("agent-room join --room 'Builders Exchange'");
    expect(message).toContain('agent-room guide');
    expect(message).toContain('%LOCALAPPDATA%\\Agent Room\\agent-room.exe');
    expect(message).not.toMatch(/--invite|--profile/u);
    expect(message).not.toMatch(appNames);
    expect(screen.queryByText(/Local agents/u)).toBeNull();
  });

  it('桌面端的命令行用实际安装路径，并把人物挂在这台电脑上等 Agent 来接', async () => {
    const writeText = clipboardMock();
    vi.stubGlobal('navigator', { clipboard: { writeText } });
    rememberMethod('cli');
    const runtime = gateway();
    renderDialog(runtime.value);

    const message = await copyMessage(writeText);
    expect(message).toContain("& 'C:\\Agent Room\\agent-room.exe' join --room 'Builders Exchange'");
    expect(message).not.toContain('%LOCALAPPDATA%');
    await waitFor(() => {
      expect(runtime.offerInvitation).toHaveBeenCalled();
    });
    expect(runtime.offerInvitation.mock.calls[0]?.[0]).toMatchObject({
      room: { catalogId: CATALOG, roomId: room.roomId },
    });
    expect(runtime.offerInvitation.mock.calls[0]?.[0]).not.toHaveProperty('displayName');
    expect(screen.getByText(/turn on background replies/u)).toBeVisible();
  });

  it('MCP：一句用 agent_room_join 的话；第一次用的配置收在折叠里', async () => {
    const writeText = clipboardMock();
    vi.stubGlobal('navigator', { clipboard: { writeText } });
    rememberMethod('mcp');
    renderDialog(gateway().value);

    const message = await copyMessage(writeText);
    expect(message).toContain('agent_room_join');
    expect(message).toContain('“Builders Exchange”');
    expect(message).not.toMatch(/sessionKey|agent_room_open_session/u);
    const setup = screen.getByText('First time using MCP? Set it up once');
    expect(setup.closest('details')).not.toHaveAttribute('open');
    expect(screen.getByText(/"agent_room"/u)).toBeInTheDocument();
    expect(screen.getByText(/just tell your agent “Join Agent Room”/u)).toBeVisible();
  });

  it('没有房间时（从“我的 Agent”打开）说进公共大厅，命令不带房间', async () => {
    const writeText = clipboardMock();
    vi.stubGlobal('navigator', { clipboard: { writeText } });
    rememberMethod('cli');
    renderDialog(gateway().value, { room: null });

    expect(screen.getByText('It joins the public lobby.')).toBeVisible();
    const message = await copyMessage(writeText);
    expect(message).toContain("& 'C:\\Agent Room\\agent-room.exe' join\n");
    expect(message).not.toContain('--room');
  });

  it('打开前就在的会话不算；之后新来的 Agent 显示进来了，并告诉房间页', async () => {
    rememberMethod('cli');
    let sessions: HostSessionDiagnostics[] = [
      session('0198b601-77a3-74f1-b4f4-940f291951d1', 'ready', { displayName: 'Old timer' }),
    ];
    const onConnected = vi.fn();
    let polls = 0;
    renderDialog(
      gateway({
        sessions: () => {
          polls += 1;
          return ok(sessions);
        },
      }).value,
      { onConnected },
    );
    // 第一次读到的会话记作已经在的；等它读过一次再让新 Agent 进来。
    await waitFor(() => {
      expect(polls).toBeGreaterThan(0);
    });

    expect(
      await screen.findByText('Send the message to your agent. It shows up here when it joins.'),
    ).toBeVisible();
    await waitFor(() => {
      expect(screen.queryByText(/Old timer/u)).toBeNull();
    });
    sessions = [...sessions, session('0198b601-77a3-74f1-b4f4-940f291951d2', 'ready')];

    expect(await screen.findByText('“Scout” joined', {}, { timeout: 5_000 })).toBeVisible();
    expect(screen.getByText('It’s reading messages.')).toBeVisible();
    expect(screen.queryByText(/Old timer/u)).toBeNull();
    await waitFor(() => {
      expect(onConnected).toHaveBeenCalledWith(
        expect.objectContaining({
          agentId: '0198b601-77a3-74f1-b4f4-940f291951e1',
          roomId: room.roomId,
          displayName: 'Scout',
        }),
      );
    });
    expect(screen.getByRole('button', { name: 'Done' })).toBeVisible();
  }, 10_000);

  it('没能进来的给出错误码；进了同一个大厅另一间的说它在别处', async () => {
    rememberMethod('mcp');
    let sessions: HostSessionDiagnostics[] = [];
    let polls = 0;
    renderDialog(
      gateway({
        sessions: () => {
          polls += 1;
          return ok(sessions);
        },
      }).value,
    );
    await waitFor(() => {
      expect(polls).toBeGreaterThan(0);
    });
    await screen.findByText('Send the message to your agent. It shows up here when it joins.');
    sessions = [
      session('0198b601-77a3-74f1-b4f4-940f291951d3', 'failed', { displayName: 'Broken' }),
      session('0198b601-77a3-74f1-b4f4-940f291951d4', 'ready', {
        displayName: 'Wanderer',
        roomId: '!other-instance:matrix.test',
        requestedRoom: { catalogId: CATALOG },
      }),
    ];

    expect(await screen.findByText('“Broken” couldn’t join', {}, { timeout: 5_000 })).toBeVisible();
    expect(screen.getByText('Error code: bridge.agent_runtime_unavailable')).toBeVisible();
    expect(screen.getByText('“Wanderer” joined another room of this lobby')).toBeVisible();
    expect(screen.queryByRole('button', { name: 'Done' })).toBeNull();
  }, 10_000);

  it('网络 Agent 看房间里新出现的人：之后来的显示进来了，原来就在的不算', async () => {
    const present = [{ agentId: '0198b601-77a3-74f1-b4f4-940f291951f1', displayName: 'Resident' }];
    const view = renderDialog(gateway({ available: false }).value, {
      presentAgents: present,
      onStartConversation: vi.fn(),
    });
    expect(screen.queryByText(/Resident/u)).toBeNull();

    view.rerenderWith({
      presentAgents: [
        ...present,
        { agentId: '0198b601-77a3-74f1-b4f4-940f291951f2', displayName: 'Web visitor' },
      ],
    });

    expect(await screen.findByText('“Web visitor” joined')).toBeVisible();
    expect(screen.queryByText(/Resident/u)).toBeNull();
    expect(screen.getByRole('button', { name: 'Start chatting' })).toBeVisible();
  });

  it('这台电脑还要授权时给授权按钮，授权前不能复制；启动中只是告知', async () => {
    vi.stubGlobal('navigator', { clipboard: { writeText: clipboardMock() } });
    rememberMethod('cli');
    renderDialog(gateway({ phase: 'authorization_required' }).value);

    expect(await screen.findByRole('button', { name: 'Authorize this computer' })).toBeVisible();
    expect(screen.getByRole('button', { name: 'Copy message' })).toBeDisabled();
    cleanup();

    renderDialog(gateway({ phase: 'starting' }).value);
    expect(await screen.findByText('Starting this computer’s connection service…')).toBeVisible();
    expect(screen.getByRole('button', { name: 'Copy message' })).toBeEnabled();
  });

  it('记住上次的方式；方向键切换；Esc 与关闭按钮都关掉', () => {
    const view = renderDialog(gateway({ available: false }).value);
    const network = screen.getByRole('radio', { name: 'Network' });
    network.focus();
    fireEvent.keyDown(network, { key: 'ArrowRight' });
    expect(screen.getByRole('radio', { name: 'MCP' })).toHaveAttribute('aria-checked', 'true');
    expect(window.localStorage.getItem('agent-room.invite-method')).toBe('mcp');

    fireEvent.click(screen.getByRole('button', { name: 'Close' }));
    expect(view.onClose).toHaveBeenCalledTimes(1);
    fireEvent(screen.getByRole('dialog'), new Event('cancel', { cancelable: true }));
    expect(view.onClose).toHaveBeenCalledTimes(2);
  });
});
