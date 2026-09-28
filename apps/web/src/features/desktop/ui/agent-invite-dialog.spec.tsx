// @vitest-environment jsdom
import '@testing-library/jest-dom/vitest';
import { cleanup, fireEvent, render, screen, waitFor } from '@testing-library/react';
import { I18nextProvider } from 'react-i18next';
import { afterEach, beforeAll, beforeEach, describe, expect, it, vi } from 'vitest';

import { readInviteHistory } from '../domain/cli-invitation';
import { AgentInviteDialog } from './agent-invite-dialog';
import { DesktopRuntimeProvider } from './desktop-runtime-provider';
import type {
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
const room = { roomId: '!builders:matrix.test', roomName: 'Builders Exchange' };
const uuidV7 = /^[0-9a-f]{8}-[0-9a-f]{4}-7[0-9a-f]{3}-[89ab][0-9a-f]{3}-[0-9a-f]{12}$/u;

function clipboardMock() {
  return vi.fn<(text: string) => Promise<void>>(() => Promise.resolve());
}
function copied(mock: ReturnType<typeof clipboardMock>, index: number): string {
  return mock.mock.calls[index]?.[0] ?? '';
}

const readyBridge: BridgeRuntime = {
  authorization: null,
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
    phase: 'ready',
  },
};

type SessionsResult = Result<
  readonly HostSessionDiagnostics[],
  { code: string; retryable: boolean }
>;

function gateway(
  options: {
    readonly available?: boolean;
    readonly sessions?: () => SessionsResult;
    /** The Bridge accepts the dialog's character as a pending invitation. */
    readonly offers?: boolean;
  } = {},
) {
  const unavailable = () => Promise.resolve(err({ code: 'test.unavailable', retryable: false }));
  const offerInvitation = vi.fn((invitation: InvitationOffer) =>
    Promise.resolve(ok({ invitation, expiresInMs: 600_000 })),
  );
  const withdrawInvitation = vi.fn<
    (sessionKey: string) => Promise<Result<void, { code: string; retryable: boolean }>>
  >(() => Promise.resolve(ok(undefined)));
  const value: DesktopRuntimeGateway = {
    beginHumanAuthentication: unavailable,
    beginMatrixAuthentication: unavailable,
    bootstrapDefaultAgent: unavailable,
    checkUpdate: unavailable,
    clearHumanSession: () => Promise.resolve(ok(undefined)),
    restoreHumanSession: () => Promise.resolve(ok(true)),
    configureAgentRuntime: (target) => Promise.resolve(ok(target)),
    installUpdate: unavailable,
    isAvailable: () => options.available ?? true,
    openAuthorization: unavailable,
    readLobby: unavailable,
    retryBridge: () => Promise.resolve(ok(readyBridge)),
    reauthorizeBridge: () => Promise.resolve(ok(readyBridge)),
    setAutostart: (enabled) => Promise.resolve(ok(enabled)),
    snapshot: () =>
      Promise.resolve(
        ok({
          agentTarget: null,
          autostartEnabled: false,
          bridge: readyBridge,
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
    ...(options.offers === true ? { offerInvitation, withdrawInvitation } : {}),
  };
  return { value, offerInvitation, withdrawInvitation };
}

function renderDialog(
  runtime: DesktopRuntimeGateway,
  props: Partial<Parameters<typeof AgentInviteDialog>[0]> = {},
) {
  const onClose = vi.fn();
  const view = render(
    <I18nextProvider i18n={i18n}>
      <DesktopRuntimeProvider gateway={runtime}>
        <AgentInviteDialog
          downloadUrl="https://download.test/agent-room.exe"
          onClose={onClose}
          owner={owner}
          room={room}
          {...props}
        />
      </DesktopRuntimeProvider>
    </I18nextProvider>,
  );
  return { ...view, onClose };
}

async function readyToCopy() {
  await waitFor(() => {
    expect(screen.getByRole('button', { name: 'Copy connection instructions' })).toBeEnabled();
  });
}
function chooseMethod(name: 'Network' | 'MCP' | 'Command line') {
  fireEvent.click(screen.getByRole('radio', { name: new RegExp(`^${name}`, 'u') }));
}
/** 接入方式存在本机；本机接入的用例从命令行或 MCP 开始。 */
function rememberMethod(method: 'network' | 'mcp' | 'cli') {
  window.localStorage.setItem('agent-room.invite-method', method);
}
const appNames = /Codex|Claude Code|Cursor|CODEX_/u;
function profileIn(prompt: string): string {
  const key = /--profile ([0-9a-f-]+)/u.exec(prompt)?.[1];
  if (key === undefined) throw new Error('Invitation has no task profile');
  return key;
}
function invitationIn(prompt: string): unknown {
  const value = /join --invite ([A-Za-z0-9_-]+)/u.exec(prompt)?.[1];
  if (value === undefined) throw new Error('Invitation missing');
  return JSON.parse(
    new TextDecoder().decode(
      Uint8Array.from(atob(value.replaceAll('-', '+').replaceAll('_', '/')), (character) =>
        character.charCodeAt(0),
      ),
    ),
  ) as unknown;
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
});

describe('AgentInviteDialog', () => {
  it('浏览器提供 CLI 邀请与下载，不伪造本机进程状态', async () => {
    const writeText = clipboardMock();
    vi.stubGlobal('navigator', { clipboard: { writeText } });
    renderDialog(gateway({ available: false }).value);
    chooseMethod('Command line');
    expect(screen.getByRole('link', { name: 'Download for Windows' })).toHaveAttribute(
      'href',
      'https://download.test/agent-room.exe',
    );
    fireEvent.click(screen.getByRole('button', { name: 'Copy connection instructions' }));
    await waitFor(() => {
      expect(writeText).toHaveBeenCalledOnce();
    });
    expect(copied(writeText, 0)).toContain('agent-room join --invite');
    expect(screen.getByText(/This browser cannot inspect/u)).toBeVisible();
    expect(
      screen.queryByText('Waiting for the agent to run its connection instructions…'),
    ).toBeNull();
  });

  it('网页上先给只凭网络接入的一句话：复制给任意 Agent，让它读 agents.md 自己进大厅', async () => {
    const writeText = clipboardMock();
    vi.stubGlobal('navigator', { clipboard: { writeText } });
    renderDialog(gateway({ available: false }).value);

    const network = screen.getByRole('region', { name: 'Just use the internet' });
    expect(network).toBeVisible();
    fireEvent.click(screen.getByRole('button', { name: 'Copy for any agent' }));
    await waitFor(() => {
      expect(writeText).toHaveBeenCalledOnce();
    });
    const prompt = copied(writeText, 0);
    expect(prompt).toContain(new URL('/agents.md', window.location.origin).href);
    expect(prompt).toContain('short, recognizable name');
    expect(prompt).toContain('untrusted input');
    expect(screen.getByRole('button', { name: 'Copied' })).toBeVisible();
  });

  it('三种接入方式平级，默认网络接入，记住上次的选择，全程不出现具体 Agent 应用的名字', async () => {
    const writeText = clipboardMock();
    vi.stubGlobal('navigator', { clipboard: { writeText } });
    const runtime = gateway();
    const first = renderDialog(runtime.value);
    expect(screen.getByRole('radio', { name: /^Network/u })).toHaveAttribute(
      'aria-checked',
      'true',
    );
    expect(screen.getByRole('radio', { name: /^MCP/u })).toHaveAttribute('aria-checked', 'false');
    expect(screen.getByRole('button', { name: 'Copy for any agent' })).toBeVisible();
    expect(screen.getByRole('dialog')).not.toHaveTextContent(appNames);

    chooseMethod('Command line');
    await readyToCopy();
    expect(screen.getByRole('dialog')).not.toHaveTextContent(appNames);
    fireEvent.click(screen.getByRole('button', { name: 'Copy connection instructions' }));
    await waitFor(() => {
      expect(writeText).toHaveBeenCalledOnce();
    });
    expect(copied(writeText, 0)).not.toMatch(appNames);
    expect(copied(writeText, 0)).toContain('register --help');
    chooseMethod('MCP');
    expect(screen.getByRole('dialog')).not.toHaveTextContent(appNames);
    first.unmount();

    renderDialog(runtime.value);
    expect(screen.getByRole('radio', { name: /^MCP/u })).toHaveAttribute('aria-checked', 'true');
  });

  it('方向键在三种接入方式间切换，焦点跟着走', () => {
    renderDialog(gateway().value);
    const network = screen.getByRole('radio', { name: /^Network/u });
    network.focus();
    fireEvent.keyDown(network, { key: 'ArrowRight' });
    const mcp = screen.getByRole('radio', { name: /^MCP/u });
    expect(mcp).toHaveAttribute('aria-checked', 'true');
    expect(mcp).toHaveFocus();
    fireEvent.keyDown(mcp, { key: 'ArrowLeft' });
    fireEvent.keyDown(screen.getByRole('radio', { name: /^Network/u }), { key: 'ArrowLeft' });
    expect(screen.getByRole('radio', { name: /^Command line/u })).toHaveAttribute(
      'aria-checked',
      'true',
    );
  });

  it('网络接入不在这台电脑上挂出邀请；换成本机方式才挂，换回来就撤回', async () => {
    const runtime = gateway({ offers: true });
    renderDialog(runtime.value);
    await new Promise((resolve) => setTimeout(resolve, 50));
    expect(runtime.offerInvitation).not.toHaveBeenCalled();
    chooseMethod('Command line');
    await waitFor(() => {
      expect(runtime.offerInvitation).toHaveBeenCalled();
    });
    const offered = runtime.offerInvitation.mock.calls[0]?.[0];
    chooseMethod('Network');
    await waitFor(() => {
      expect(runtime.withdrawInvitation).toHaveBeenCalledWith(offered?.sessionKey);
    });
  });

  it('命令行无需 MCP 配置；新任务独立，旧人物可明确恢复', async () => {
    const writeText = clipboardMock();
    vi.stubGlobal('navigator', { clipboard: { writeText } });
    rememberMethod('cli');
    const { value } = gateway();
    const first = renderDialog(value);
    await readyToCopy();
    // 名字默认留给 Agent 自己起：邀请里不带名字，说明里请它用 --name 起一个。
    const name = screen.getByLabelText('Agent name');
    expect(name).toHaveValue('');
    expect(name).toHaveAttribute('placeholder', 'The agent names itself');
    fireEvent.click(screen.getByRole('button', { name: 'Copy connection instructions' }));
    await screen.findByText('Copied. Paste it to your agent.');
    const prompt = copied(writeText, 0);
    const key = profileIn(prompt);
    expect(key).toMatch(uuidV7);
    expect(invitationIn(prompt)).toEqual({
      version: 1,
      sessionKey: key,
      roomId: room.roomId,
    });
    expect(prompt).toContain('Add --name "…" to this command with a short, recognizable name');
    expect(prompt).toContain("& 'C:\\Agent Room\\agent-room.exe'");
    expect(prompt).toContain('ack --event');
    expect(prompt).toContain('single command blocks silently until a message arrives');
    expect(prompt).not.toContain('read --wait 25');
    expect(prompt).toContain('untrusted input');
    expect(prompt).toContain('do not claim to still be listening');
    first.unmount();

    renderDialog(value);
    await readyToCopy();
    fireEvent.click(screen.getByRole('button', { name: 'Copy connection instructions' }));
    await waitFor(() => {
      expect(writeText).toHaveBeenCalledTimes(2);
    });
    expect(profileIn(copied(writeText, 1))).not.toBe(key);
    fireEvent.change(screen.getByLabelText('Saved characters'), { target: { value: key } });
    expect(screen.getByLabelText('Agent name')).toBeDisabled();
    fireEvent.click(screen.getByRole('button', { name: 'Copy connection instructions' }));
    await waitFor(() => {
      expect(writeText).toHaveBeenCalledTimes(3);
    });
    expect(copied(writeText, 2)).toBe(prompt);
  });

  it('面板开着就挂出这个人物：一句“接入 Agent Room”即可，记下 Agent 自己起的名字，关闭时撤回', async () => {
    let sessions: HostSessionDiagnostics[] = [];
    rememberMethod('cli');
    const runtime = gateway({ offers: true, sessions: () => ok(sessions) });
    const view = renderDialog(runtime.value);
    await readyToCopy();
    await waitFor(() => {
      expect(runtime.offerInvitation).toHaveBeenCalled();
    });
    const offered = runtime.offerInvitation.mock.calls[0]?.[0];
    expect(offered?.sessionKey).toMatch(uuidV7);
    // 名字留给接上的 Agent 自己起。
    expect(offered?.displayName).toBeUndefined();
    // 这个房间没有目录信息，就不带房间，由 Bridge 走默认大厅。
    expect(offered?.room).toBeUndefined();
    expect(
      await screen.findByText(/tell an agent that already has Agent Room’s MCP set up/u),
    ).toBeVisible();

    // Agent 说了“接入”，用自己起的名字接上了这个人物。
    sessions = [
      {
        displayName: 'Scout',
        session: {
          sessionId: '0198b601-77a1-7bb8-83eb-a8fe68c97e50',
          state: 'ready',
          agentId: null,
          errorCode: null,
        },
        sessionKey: offered?.sessionKey ?? null,
        lastInboxReadAgoMs: 1_000,
        lastMessageReceivedAgoMs: null,
        lastMessageSentAgoMs: null,
      },
    ];
    await screen.findByText('“Scout” is in the room', undefined, { timeout: 5_000 });
    const name = screen.getByLabelText('Agent name');
    expect(name).toBeDisabled();
    expect(name).toHaveValue('Scout');
    // 记下它实际用的名字：以后恢复这个人物时沿用同一个名字。
    await waitFor(() => {
      expect(readInviteHistory(window.localStorage, owner.principalId).identities[0]).toMatchObject(
        { sessionKey: offered?.sessionKey, displayName: 'Scout' },
      );
    });
    view.unmount();
    expect(runtime.withdrawInvitation).toHaveBeenCalledWith(offered?.sessionKey);
  }, 15_000);

  it('名字可以留空交给 Agent；复制后锁定身份，切换协议不会创建另一个人物', async () => {
    const writeText = clipboardMock();
    vi.stubGlobal('navigator', { clipboard: { writeText } });
    rememberMethod('cli');
    renderDialog(gateway().value);
    await readyToCopy();
    const name = screen.getByLabelText('Agent name');
    // 只有空格等于没填：由 Agent 自己起名，仍可复制。
    fireEvent.change(name, { target: { value: '   ' } });
    expect(name).toHaveAttribute('aria-invalid', 'false');
    expect(screen.getByRole('button', { name: 'Copy connection instructions' })).toBeEnabled();
    fireEvent.change(name, { target: { value: 'Sc\u0007out' } });
    expect(name).toHaveAttribute('aria-invalid', 'true');
    expect(screen.getByRole('button', { name: 'Copy connection instructions' })).toBeDisabled();
    fireEvent.change(name, { target: { value: 'Scout' } });
    fireEvent.click(screen.getByRole('button', { name: 'Copy connection instructions' }));
    await waitFor(() => {
      expect(writeText).toHaveBeenCalledOnce();
    });
    const key = profileIn(copied(writeText, 0));
    expect(name).toBeDisabled();
    chooseMethod('MCP');
    await readyToCopy();
    fireEvent.click(screen.getByRole('button', { name: 'Copy connection instructions' }));
    await waitFor(() => {
      expect(writeText).toHaveBeenCalledTimes(2);
    });
    expect(copied(writeText, 1)).toContain('sessionKey = ' + key);
    expect(copied(writeText, 1)).toContain('displayName = Scout');
    fireEvent.click(screen.getByRole('button', { name: 'Invite another agent' }));
    expect(screen.getByLabelText('Agent name')).toBeEnabled();
    expect(screen.getByLabelText('Agent name')).toHaveValue('');
    fireEvent.click(screen.getByRole('button', { name: 'Copy connection instructions' }));
    await waitFor(() => {
      expect(writeText).toHaveBeenCalledTimes(3);
    });
    expect(copied(writeText, 2)).not.toContain('sessionKey = ' + key);
    // 没填名字：请 Agent 用自己起的名字，而不是替它编一个。
    expect(copied(writeText, 2)).toContain(
      'displayName = <a short, recognizable name you pick for yourself>',
    );
  });

  it('只有携带本次 sessionKey 的会话才算进入房间，然后可以完成', async () => {
    rememberMethod('cli');
    vi.stubGlobal('navigator', { clipboard: { writeText: () => Promise.resolve() } });
    let sessions: HostSessionDiagnostics[] = [];
    const view = renderDialog(gateway({ sessions: () => ok(sessions) }).value);
    await readyToCopy();
    fireEvent.click(screen.getByRole('button', { name: 'Copy connection instructions' }));
    await screen.findByText('Waiting for the agent to run its connection instructions…');
    const pasteHint =
      'Instructions copied. Paste them into the agent task and let it run the command.';
    expect(screen.getByText(pasteHint)).toBeVisible();
    const stored = readInviteHistory(window.localStorage, owner.principalId).identities[0];
    if (stored === undefined) throw new Error('Invitation was not saved');
    const entry = (state: HostSessionDiagnostics['session']['state'], key: string) => ({
      displayName: 'Ada’s Codex',
      session: {
        sessionId: '0198b601-77a1-7bb8-83eb-a8fe68c97e50',
        state,
        agentId: null,
        errorCode: state === 'failed' ? 'bridge.session.denied' : null,
      },
      sessionKey: key,
      lastInboxReadAgoMs: 1_000,
      lastMessageReceivedAgoMs: null,
      lastMessageSentAgoMs: null,
    });
    sessions = [entry('ready', '0198b601-77a1-7bb8-83eb-a8fe68c97e99')];
    await new Promise((resolve) => setTimeout(resolve, 3_100));
    expect(
      screen.getByText('Waiting for the agent to run its connection instructions…'),
    ).toBeVisible();

    sessions = [entry('starting', stored.sessionKey)];
    await screen.findByText('“Ada’s Codex” is entering the room…', undefined, { timeout: 5_000 });
    sessions = [entry('ready', stored.sessionKey)];
    await screen.findByText('“Ada’s Codex” is in the room', undefined, { timeout: 5_000 });
    expect(screen.getByText('Reading messages right now')).toBeVisible();
    // The paste instruction is done once the agent is in the room.
    expect(screen.queryByText(pasteHint)).toBeNull();
    fireEvent.click(screen.getByRole('button', { name: 'Done' }));
    expect(view.onClose).toHaveBeenCalledTimes(1);
  }, 15_000);

  it('接入失败时显示错误码并告知不要换身份重试；诊断不可用时不伪造等待', async () => {
    rememberMethod('cli');
    const failed: HostSessionDiagnostics = {
      displayName: 'Ada’s Codex',
      session: {
        sessionId: '0198b601-77a1-7bb8-83eb-a8fe68c97e50',
        state: 'failed',
        agentId: null,
        errorCode: 'bridge.session.denied',
      },
      sessionKey: null,
      lastInboxReadAgoMs: null,
      lastMessageReceivedAgoMs: null,
      lastMessageSentAgoMs: null,
    };
    window.localStorage.setItem(
      'agent-room.agent-invite.codex',
      JSON.stringify({
        sessionKey: '0198b601-77a1-7bb8-83eb-a8fe68c97e44',
        displayName: 'Ada’s Codex',
        ownerId: owner.principalId,
      }),
    );
    const first = renderDialog(
      gateway({
        sessions: () => ok([{ ...failed, sessionKey: '0198b601-77a1-7bb8-83eb-a8fe68c97e44' }]),
      }).value,
    );
    fireEvent.change(screen.getByLabelText('Saved characters'), {
      target: { value: '0198b601-77a1-7bb8-83eb-a8fe68c97e44' },
    });
    await screen.findByText('“Ada’s Codex” could not connect');
    expect(screen.getByText('bridge.session.denied')).toBeVisible();
    expect(screen.getByRole('alert')).toHaveTextContent('Do not retry with a different identity');
    first.unmount();

    renderDialog(
      gateway({ sessions: () => err({ code: 'bridge.ipc.bridge_unavailable', retryable: true }) })
        .value,
    );
    await screen.findByText(/Cannot check task connections right now/u);
    expect(screen.getByText(/bridge\.ipc\.bridge_unavailable/u)).toBeVisible();
    expect(
      screen.queryByText('Waiting for the agent to run its connection instructions…'),
    ).toBeNull();
  });

  it('MCP 方式只给通用配置：任何支持 MCP 的工具照着添加，不按应用区分', async () => {
    const writeText = clipboardMock();
    vi.stubGlobal('navigator', { clipboard: { writeText } });
    rememberMethod('mcp');
    renderDialog(gateway().value);
    expect(await screen.findByText(/Add this JSON to the tool’s MCP configuration/u)).toBeVisible();
    fireEvent.click(screen.getByRole('button', { name: 'Copy JSON' }));
    await waitFor(() => {
      expect(writeText).toHaveBeenCalledWith(expect.stringContaining('"agent_room"'));
    });
    expect(writeText).toHaveBeenCalledWith(expect.stringContaining('agent-room-mcp.exe'));
    expect(screen.getByRole('button', { name: 'Copied' })).toBeVisible();
    await readyToCopy();
    expect(screen.getAllByRole('radio')).toHaveLength(3);
    expect(screen.getByRole('dialog')).not.toHaveTextContent(appNames);
  });

  it('本机连接未就绪时明确提示，而不是让用户白等', async () => {
    rememberMethod('cli');
    const runtime = gateway().value;
    const starting: BridgeRuntime = {
      ...readyBridge,
      lifecycle: { ...readyBridge.lifecycle, phase: 'authorization_required' },
    };
    renderDialog({
      ...runtime,
      snapshot: async () => {
        const base = await runtime.snapshot();
        return base.ok ? ok({ ...base.value, bridge: starting }) : base;
      },
    });
    expect(await screen.findByText(/Allow this computer to connect your agents/u)).toBeVisible();
    expect(screen.getByRole('button', { name: 'Copy connection instructions' })).toBeDisabled();
    expect(
      screen.queryByText('Waiting for the agent to run its connection instructions…'),
    ).toBeNull();
  });

  it('已授权且没有默认 Agent 时可以直接接入，不要求再次授权', async () => {
    rememberMethod('cli');
    const runtime = gateway().value;
    renderDialog({
      ...runtime,
      snapshot: async () => {
        const value = await runtime.snapshot();
        return value.ok
          ? ok({
              ...value.value,
              bridge: {
                ...readyBridge,
                lifecycle: { ...readyBridge.lifecycle, phase: 'authorized' },
              },
            })
          : value;
      },
    });
    await readyToCopy();
    expect(screen.getByRole('button', { name: 'Copy connection instructions' })).toBeEnabled();
    expect(screen.queryByText(/Finish authorization/u)).toBeNull();
    expect(screen.getByText(/Ready\. Copy the instructions above/u)).toBeVisible();
  });

  it('重连可以在弹窗内重试，恢复后直接继续无需重新登录', async () => {
    rememberMethod('cli');
    const runtime = gateway().value;
    const retryBridge = vi.fn(() => runtime.retryBridge());
    renderDialog({
      ...runtime,
      retryBridge,
      snapshot: async () => {
        const value = await runtime.snapshot();
        return value.ok
          ? ok({
              ...value.value,
              bridge: {
                ...readyBridge,
                lifecycle: { ...readyBridge.lifecycle, phase: 'reconnecting' },
              },
            })
          : value;
      },
    });
    await screen.findByText(/Reconnecting automatically/u);
    expect(screen.getByRole('button', { name: 'Copy connection instructions' })).toBeDisabled();
    fireEvent.click(screen.getByRole('button', { name: 'Retry connection' }));
    await waitFor(() => {
      expect(screen.getByRole('button', { name: 'Copy connection instructions' })).toBeEnabled();
    });
    expect(retryBridge).toHaveBeenCalledOnce();
    expect(screen.queryByText(/Reconnecting automatically/u)).toBeNull();
  });
});
