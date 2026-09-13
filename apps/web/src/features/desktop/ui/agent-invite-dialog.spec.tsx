// @vitest-environment jsdom
import '@testing-library/jest-dom/vitest';
import {
  act,
  cleanup,
  fireEvent,
  render,
  renderHook,
  screen,
  waitFor,
} from '@testing-library/react';
import { I18nextProvider } from 'react-i18next';
import { afterEach, beforeAll, beforeEach, describe, expect, it, vi } from 'vitest';

import { readInviteHistory } from '../domain/cli-invitation';
import { AgentInviteDialog } from './agent-invite-dialog';
import { DesktopRuntimeProvider } from './desktop-runtime-provider';
import { useDesktopRuntime } from './use-desktop-runtime';
import type {
  BridgeRuntime,
  DesktopRuntimeGateway,
  HostSessionDiagnostics,
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
    readonly installed?: readonly ('codex' | 'claude-code' | 'cursor')[];
    readonly configured?: boolean;
  } = {},
) {
  const unavailable = () => Promise.resolve(err({ code: 'test.unavailable', retryable: false }));
  let configured = options.configured ?? true;
  const applyHost = vi.fn(() => {
    configured = true;
    return Promise.resolve(ok(undefined));
  });
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
    detectHosts: () =>
      Promise.resolve(
        ok(
          (['codex', 'claude-code', 'cursor'] as const).map((host) => ({
            host,
            installed: (options.installed ?? ['codex']).includes(host),
            configurable: (options.installed ?? ['codex']).includes(host),
            mechanism: 'config-file',
            diagnosticCode: 'test.detected',
          })),
        ),
      ),
    readHostSessions: () => Promise.resolve(options.sessions?.() ?? ok([])),
    planHost: (host) =>
      Promise.resolve(
        ok({
          host,
          action: configured ? 'unchanged' : 'create',
          target: 'config',
          originalDigest: '0'.repeat(64),
          desiredDigest: '1'.repeat(64),
          summaryCode: 'test.plan',
        }),
      ),
    applyHost,
  };
  return { value, applyHost };
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
function selectMcp() {
  fireEvent.click(screen.getByText('Other connection options'));
  fireEvent.click(screen.getByRole('radio', { name: 'MCP compatibility' }));
}
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
  it('迟到的配置检查不能覆盖已经成功的一键配置', async () => {
    const runtime = gateway({ configured: false }).value;
    const fallbackPlan = runtime.planHost?.bind(runtime);
    if (fallbackPlan === undefined) throw new Error('Fixture requires planHost');
    const delayed =
      Promise.withResolvers<Awaited<ReturnType<NonNullable<DesktopRuntimeGateway['planHost']>>>>();
    const planHost = vi
      .fn<NonNullable<DesktopRuntimeGateway['planHost']>>()
      .mockImplementationOnce(() => delayed.promise)
      .mockImplementation(fallbackPlan);
    const runtimeWithDelayedCheck = { ...runtime, planHost };
    const { result } = renderHook(() => useDesktopRuntime(runtimeWithDelayedCheck));
    let checking: Promise<void>;
    act(() => {
      checking = result.current.checkHost('codex');
    });
    await act(() => result.current.configureHost('codex'));
    expect(result.current.hostSetup.codex?.phase).toBe('configured');
    await act(async () => {
      delayed.resolve(err({ code: 'codex.list_failed', retryable: true }));
      await checking;
    });
    expect(result.current.hostSetup.codex?.phase).toBe('configured');
  });

  it('浏览器提供 CLI 邀请与下载，不伪造本机进程状态', async () => {
    const writeText = clipboardMock();
    vi.stubGlobal('navigator', { clipboard: { writeText } });
    renderDialog(gateway({ available: false }).value);
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

  it('默认 CLI 无需 MCP 配置；新任务独立，旧人物可明确恢复', async () => {
    const writeText = clipboardMock();
    vi.stubGlobal('navigator', { clipboard: { writeText } });
    const runtime = gateway({ configured: false });
    const planHost = vi.fn(runtime.value.planHost?.bind(runtime.value));
    const value = { ...runtime.value, planHost };
    const first = renderDialog(value);
    await readyToCopy();
    expect(screen.queryByRole('button', { name: 'Set up Codex in one click' })).toBeNull();
    expect(screen.getByLabelText('Agent name')).toHaveValue('Ada’s agent');
    fireEvent.click(screen.getByRole('button', { name: 'Copy connection instructions' }));
    await screen.findByText('Copied. Paste it to your agent.');
    const prompt = copied(writeText, 0);
    const key = profileIn(prompt);
    expect(key).toMatch(uuidV7);
    expect(invitationIn(prompt)).toEqual({
      version: 1,
      sessionKey: key,
      displayName: 'Ada’s agent',
      roomId: room.roomId,
    });
    expect(prompt).toContain("& 'C:\\Agent Room\\agent-room.exe'");
    expect(prompt).toContain('ack --event');
    expect(prompt).toContain('single command blocks silently until a message arrives');
    expect(prompt).not.toContain('read --wait 25');
    expect(prompt).toContain('untrusted input');
    expect(prompt).toContain('do not claim to still be listening');
    expect(planHost).not.toHaveBeenCalled();
    expect(runtime.applyHost).not.toHaveBeenCalled();
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

  it('空名字不能复制；复制后锁定身份，切换协议不会创建另一个人物', async () => {
    const writeText = clipboardMock();
    vi.stubGlobal('navigator', { clipboard: { writeText } });
    renderDialog(gateway({ installed: ['codex', 'cursor'] }).value);
    await readyToCopy();
    const name = screen.getByLabelText('Agent name');
    fireEvent.change(name, { target: { value: '   ' } });
    expect(name).toHaveAttribute('aria-invalid', 'true');
    expect(screen.getByRole('button', { name: 'Copy connection instructions' })).toBeDisabled();
    fireEvent.change(name, { target: { value: 'Scout' } });
    fireEvent.click(screen.getByRole('button', { name: 'Copy connection instructions' }));
    await waitFor(() => {
      expect(writeText).toHaveBeenCalledOnce();
    });
    const key = profileIn(copied(writeText, 0));
    expect(name).toBeDisabled();
    selectMcp();
    await screen.findByText(/Codex is set up\./u);
    fireEvent.click(screen.getByRole('radio', { name: 'Cursor' }));
    await screen.findByText(/Cursor is set up\./u);
    fireEvent.click(screen.getByRole('button', { name: 'Copy connection instructions' }));
    await waitFor(() => {
      expect(writeText).toHaveBeenCalledTimes(2);
    });
    expect(copied(writeText, 1)).toContain('sessionKey = ' + key);
    expect(copied(writeText, 1)).toContain('displayName = Scout');
    fireEvent.click(screen.getByRole('button', { name: 'Invite another agent' }));
    expect(screen.getByLabelText('Agent name')).toBeEnabled();
    fireEvent.click(screen.getByRole('button', { name: 'Copy connection instructions' }));
    await waitFor(() => {
      expect(writeText).toHaveBeenCalledTimes(3);
    });
    expect(copied(writeText, 2)).not.toContain('sessionKey = ' + key);
  });

  it('只有携带本次 sessionKey 的会话才算进入房间，然后可以完成', async () => {
    vi.stubGlobal('navigator', { clipboard: { writeText: () => Promise.resolve() } });
    let sessions: HostSessionDiagnostics[] = [];
    const view = renderDialog(gateway({ sessions: () => ok(sessions) }).value);
    await readyToCopy();
    fireEvent.click(screen.getByRole('button', { name: 'Copy connection instructions' }));
    await screen.findByText('Waiting for the agent to run its connection instructions…');
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
    fireEvent.click(screen.getByRole('button', { name: 'Done' }));
    expect(view.onClose).toHaveBeenCalledTimes(1);
  }, 15_000);

  it('接入失败时显示错误码并告知不要换身份重试；诊断不可用时不伪造等待', async () => {
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

  it('一键配置已安装的宿主，未安装的宿主给出说明，其他工具提供可复制的 JSON', async () => {
    const writeText = clipboardMock();
    vi.stubGlobal('navigator', { clipboard: { writeText } });
    const runtime = gateway({ configured: false });
    renderDialog(runtime.value);
    selectMcp();
    fireEvent.click(await screen.findByRole('button', { name: 'Set up Codex in one click' }));
    await waitFor(() => {
      expect(runtime.applyHost).toHaveBeenCalledWith('codex', '0'.repeat(64));
    });
    expect(await screen.findByText(/Codex is set up\./u)).toBeVisible();

    fireEvent.click(screen.getByRole('radio', { name: /Claude Code/u }));
    expect(screen.getByText(/Claude Code was not detected on this computer/u)).toBeVisible();

    fireEvent.click(screen.getByRole('radio', { name: 'Other MCP tool' }));
    expect(screen.getByText(/Add this JSON to the tool’s MCP configuration/u)).toBeVisible();
    fireEvent.click(screen.getByRole('button', { name: 'Copy JSON' }));
    await waitFor(() => {
      expect(writeText).toHaveBeenCalledWith(expect.stringContaining('"agent_room"'));
    });
    expect(writeText).toHaveBeenCalledWith(expect.stringContaining('agent-room-mcp.exe'));
  });

  it('本机连接未就绪时明确提示，而不是让用户白等', async () => {
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

  it('旧 Codex 配置错误解释原因并阻止伪造等待，切换宿主不携带旧错误', async () => {
    const runtime = gateway().value;
    renderDialog({
      ...runtime,
      planHost: () => Promise.resolve(err({ code: 'codex.config_incompatible', retryable: true })),
    });
    await readyToCopy();
    selectMcp();
    expect(await screen.findByText(/cannot read your current settings/u)).toBeVisible();
    expect(screen.getByRole('button', { name: 'Copy connection instructions' })).toBeDisabled();
    expect(
      screen.queryByText('Waiting for the agent to run its connection instructions…'),
    ).toBeNull();
    fireEvent.click(screen.getByRole('radio', { name: 'Other MCP tool' }));
    expect(screen.queryByText(/cannot read your current settings/u)).toBeNull();
  });

  it('重连可以在弹窗内重试，恢复后直接继续无需重新登录', async () => {
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
