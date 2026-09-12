// @vitest-environment jsdom
import '@testing-library/jest-dom/vitest';
import { cleanup, fireEvent, render, screen, waitFor } from '@testing-library/react';
import { I18nextProvider } from 'react-i18next';
import { afterEach, beforeAll, beforeEach, describe, expect, it, vi } from 'vitest';

import { AgentInviteDialog } from './agent-invite-dialog';
import { DesktopRuntimeProvider } from './desktop-runtime-provider';
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
  } = {},
) {
  const unavailable = () => Promise.resolve(err({ code: 'test.unavailable', retryable: false }));
  const applyHost = vi.fn(() => Promise.resolve(ok(undefined)));
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
          action: 'create',
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
  it('浏览器里没有桌面运行时时，指向运行 Agent 的电脑并提供下载', () => {
    renderDialog(gateway({ available: false }).value);
    expect(screen.getByRole('heading', { name: 'Bring an agent into the room' })).toBeVisible();
    expect(screen.getByText('Finish this on the computer that runs your agent')).toBeVisible();
    expect(screen.getByRole('link', { name: 'Download for Windows' })).toHaveAttribute(
      'href',
      'https://download.test/agent-room.exe',
    );
    expect(screen.queryByRole('button', { name: 'Copy connection instructions' })).toBeNull();
  });

  it('复制的指令带专属身份和当前房间，身份持久化后再次打开保持不变', async () => {
    const writeText = clipboardMock();
    vi.stubGlobal('navigator', { clipboard: { writeText } });
    const runtime = gateway().value;
    const first = renderDialog(runtime);
    await waitFor(() => {
      expect(screen.getByRole('radio', { name: /Codex/u })).toHaveAttribute('aria-checked', 'true');
    });
    expect(screen.getByLabelText('Agent name')).toHaveValue('Ada’s Codex');
    fireEvent.click(screen.getByRole('button', { name: 'Copy connection instructions' }));
    await waitFor(() => {
      expect(screen.getByText('Copied. Paste it to your agent.')).toBeVisible();
    });
    const prompt = copied(writeText, 0);
    const key = /sessionKey = (\S+)/u.exec(prompt)?.[1];
    expect(key).toMatch(uuidV7);
    expect(prompt).toContain('displayName = Ada’s Codex');
    expect(prompt).toContain('roomId = !builders:matrix.test (Builders Exchange)');
    expect(prompt).toContain('untrusted input');
    expect(prompt).toContain('do not claim to still be listening');
    expect(screen.getByText('Waiting for it to call the Agent Room tools…')).toBeVisible();

    first.unmount();
    renderDialog(runtime);
    fireEvent.click(await screen.findByRole('button', { name: 'Copy connection instructions' }));
    await waitFor(() => {
      expect(writeText).toHaveBeenCalledTimes(2);
    });
    expect(copied(writeText, 1)).toContain(`sessionKey = ${key ?? ''}`);
  });

  it('改名和换新身份都会更新指令；不同宿主使用不同身份', async () => {
    const writeText = clipboardMock();
    vi.stubGlobal('navigator', { clipboard: { writeText } });
    renderDialog(gateway({ installed: ['codex', 'cursor'] }).value);
    await waitFor(() => {
      expect(screen.getByRole('radio', { name: /Codex/u })).toHaveAttribute('aria-checked', 'true');
    });
    const name = screen.getByLabelText('Agent name');
    fireEvent.change(name, { target: { value: '  Scout  ' } });
    fireEvent.click(screen.getByRole('button', { name: 'Copy connection instructions' }));
    await waitFor(() => {
      expect(writeText).toHaveBeenCalledTimes(1);
    });
    const firstPrompt = copied(writeText, 0);
    expect(firstPrompt).toContain('displayName = Scout');
    const firstKey = /sessionKey = (\S+)/u.exec(firstPrompt)?.[1];

    fireEvent.change(name, { target: { value: '   ' } });
    expect(screen.getByText('Use 1 to 128 characters, not only spaces.')).toBeVisible();
    expect(name).toHaveAttribute('aria-invalid', 'true');

    fireEvent.click(screen.getByRole('button', { name: 'Use a new identity' }));
    fireEvent.click(screen.getByRole('button', { name: 'Copy connection instructions' }));
    await waitFor(() => {
      expect(writeText).toHaveBeenCalledTimes(2);
    });
    const secondPrompt = copied(writeText, 1);
    expect(secondPrompt).toContain('displayName = Scout');
    expect(/sessionKey = (\S+)/u.exec(secondPrompt)?.[1]).not.toBe(firstKey);

    fireEvent.click(screen.getByRole('radio', { name: /Cursor/u }));
    expect(screen.getByLabelText('Agent name')).toHaveValue('Ada’s Cursor');
    fireEvent.click(screen.getByRole('button', { name: 'Copy connection instructions' }));
    await waitFor(() => {
      expect(writeText).toHaveBeenCalledTimes(3);
    });
    expect(/sessionKey = (\S+)/u.exec(copied(writeText, 2))?.[1]).not.toBe(
      /sessionKey = (\S+)/u.exec(secondPrompt)?.[1],
    );
  });

  it('只有携带本次 sessionKey 的会话才算进入房间，然后可以完成', async () => {
    vi.stubGlobal('navigator', { clipboard: { writeText: () => Promise.resolve() } });
    const storageKey = 'agent-room.agent-invite.codex';
    let sessions: HostSessionDiagnostics[] = [];
    const view = renderDialog(gateway({ sessions: () => ok(sessions) }).value);
    await screen.findByText('Waiting for it to call the Agent Room tools…');
    const stored = JSON.parse(window.localStorage.getItem(storageKey) ?? '{}') as {
      sessionKey: string;
    };
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
    expect(screen.getByText('Waiting for it to call the Agent Room tools…')).toBeVisible();

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
    expect(screen.queryByText('Waiting for it to call the Agent Room tools…')).toBeNull();
  });

  it('一键配置已安装的宿主，未安装的宿主给出说明，其他工具提供可复制的 JSON', async () => {
    const writeText = clipboardMock();
    vi.stubGlobal('navigator', { clipboard: { writeText } });
    const runtime = gateway();
    renderDialog(runtime.value);
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
    expect(
      await screen.findByText(
        /The local connection is not ready yet: Local access needs authorization/u,
      ),
    ).toBeVisible();
  });
});
