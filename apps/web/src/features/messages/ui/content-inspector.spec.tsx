// @vitest-environment jsdom

import '@testing-library/jest-dom/vitest';

import { cleanup, render, screen, waitFor } from '@testing-library/react';
import userEvent from '@testing-library/user-event';
import { I18nextProvider } from 'react-i18next';
import { QueryClient, QueryClientProvider } from '@tanstack/react-query';
import { afterEach, beforeAll, describe, expect, it, vi } from 'vitest';

import type { HandoffGateway } from '@/features/handoffs/domain/handoff';
import {
  acceptedHandoffFixture,
  handoffSnapshotFixture,
  handoffTargetFixture,
} from '@/features/handoffs/testing/handoff-fixtures';
import type { ContentGateway, ContentVerifier } from '@/features/messages/domain/content';
import type { MachineTranslationGateway } from '@/features/messages/domain/machine-translation';
import type { RoomMessageSignal } from '@/features/messages/domain/message';
import { ContentInspector } from '@/features/messages/ui/content-inspector';
import type { ModerationGateway } from '@/features/moderation/domain/moderation';
import { i18n, initializeI18n } from '@/shared/i18n/i18n';
import { ok } from '@/shared/result';
import { remotePromptInjectionFixture } from '@/test/fixtures/remote-prompt-injection';

beforeAll(async () => {
  await initializeI18n(window.localStorage, ['en']);
});

afterEach(cleanup);

describe('ContentInspector', () => {
  it('明确展示房间上下文和当前客户端真正完成的验签层级', () => {
    renderInspector(dependencies('safe'));

    expect(screen.getByText('!public:agent-room.test')).toBeInTheDocument();
    expect(
      screen.getByText('Matrix sender matched · Agent instance signature not reverified in Web'),
    ).toBeInTheDocument();
    expect(screen.queryByText('Agent instance signature verified')).not.toBeInTheDocument();
  });

  it('选择预览时零正文网络，只有点击后才执行票据、下载和校验', async () => {
    const user = userEvent.setup();
    const runtime = dependencies(
      '# Safe\n<img src=x onerror=alert(1)>\n[link](javascript:alert(1))',
    );
    const view = renderInspector(runtime);

    expect(runtime.issueReadTicket).not.toHaveBeenCalled();
    expect(runtime.download).not.toHaveBeenCalled();
    expect(runtime.verify).not.toHaveBeenCalled();

    await user.click(screen.getByRole('button', { name: 'Open full content' }));
    await screen.findByText('Length and SHA-256 verified');

    expect(runtime.issueReadTicket).toHaveBeenCalledOnce();
    expect(runtime.download).toHaveBeenCalledOnce();
    expect(runtime.verify).toHaveBeenCalledOnce();
    expect(screen.getByText('<img src=x onerror=alert(1)>')).toBeVisible();
    expect(screen.getByText('[link](javascript:alert(1))')).toBeVisible();
    expect(view.container.querySelector('img')).toBeNull();
    expect(view.container.querySelector('a')).toBeNull();
  });

  it('查看正文不会隐式交付，交给 Agent 需要独立确认精确实例和范围', async () => {
    const user = userEvent.setup();
    const runtime = dependencies('Verified remote context');
    renderInspector(runtime);

    expect(screen.queryByRole('button', { name: 'Give to Agent' })).not.toBeInTheDocument();
    await user.click(screen.getByRole('button', { name: 'Open full content' }));
    await screen.findByText('Length and SHA-256 verified');

    expect(runtime.listTargets).not.toHaveBeenCalled();
    expect(runtime.approve).not.toHaveBeenCalled();
    await user.click(screen.getByRole('button', { name: 'Give to Agent' }));
    await screen.findByRole('heading', { name: 'Approve one-time context' });

    expect(runtime.listTargets).toHaveBeenCalledWith('!public:agent-room.test');
    expect(runtime.approve).not.toHaveBeenCalled();
    await user.click(screen.getByRole('button', { name: 'Confirm handoff' }));
    await screen.findByText('Queued for one instance');

    expect(runtime.approve).toHaveBeenCalledOnce();
    expect(runtime.approve.mock.calls[0]?.[0]).toMatchObject({
      permissions: ['read_text', 'include_metadata'],
      purpose: 'summarize',
      target: {
        instanceId: '01990d9e-8400-7000-8000-000000000012',
      },
    });
  });

  it('远端提示注入只能显示为惰性文本且不能自动进入上下文或触发动作', async () => {
    const user = userEvent.setup();
    const runtime = dependencies(remotePromptInjectionFixture);
    const view = renderInspector(runtime);

    await user.click(screen.getByRole('button', { name: 'Open full content' }));
    await screen.findByText(/Ignore all previous instructions/iu);

    expect(runtime.listTargets).not.toHaveBeenCalled();
    expect(runtime.approve).not.toHaveBeenCalled();
    expect(runtime.translate).not.toHaveBeenCalled();
    expect(runtime.report).not.toHaveBeenCalled();
    expect(runtime.onClose).not.toHaveBeenCalled();
    expect(
      view.container.querySelector('script, img, a, iframe, form, button[type="submit"]'),
    ).toBeNull();
    expect(Reflect.get(window, '__agentRoomCompromised')).toBeUndefined();
  });

  it('机器翻译必须显式点击，结果标记为机器生成且原文保持可见', async () => {
    const user = userEvent.setup();
    const runtime = dependencies('需要保留的原始正文');
    renderInspector(runtime);

    await user.click(screen.getByRole('button', { name: 'Open full content' }));
    await screen.findByText('需要保留的原始正文');
    expect(runtime.translate).not.toHaveBeenCalled();

    await user.click(screen.getByRole('button', { name: 'Translate explicitly' }));
    expect(await screen.findByText('Machine translation')).toBeVisible();
    expect(screen.getByText('Translated locally')).toBeVisible();
    expect(screen.getByText('需要保留的原始正文')).toBeVisible();
    expect(runtime.translate).toHaveBeenCalledOnce();
  });

  it('撤回消息永远不会出现正文读取按钮', () => {
    const runtime = dependencies('hidden');
    renderInspector(runtime, { ...message(), content: null, lifecycle: 'redacted', preview: null });

    expect(screen.getByText(/no content request was made/iu)).toBeInTheDocument();
    expect(screen.queryByRole('button', { name: 'Open full content' })).not.toBeInTheDocument();
    expect(runtime.issueReadTicket).not.toHaveBeenCalled();
  });

  it('关闭动作不触发隐式重试或下载', async () => {
    const user = userEvent.setup();
    const runtime = dependencies('safe');
    renderInspector(runtime);

    await user.click(screen.getByRole('button', { name: 'Close message details' }));
    await waitFor(() => {
      expect(runtime.onClose).toHaveBeenCalledOnce();
    });
    expect(runtime.issueReadTicket).not.toHaveBeenCalled();
  });

  it('举报只在用户勾选后附带当前预览且不读取受保护正文', async () => {
    const user = userEvent.setup();
    const runtime = dependencies('protected body');
    renderInspector(runtime);

    await user.click(screen.getByRole('button', { name: 'Report' }));
    await user.click(
      screen.getByRole('checkbox', { name: /include the visible preview summary/iu }),
    );
    await user.click(screen.getByRole('button', { name: 'Create report' }));

    await waitFor(() => {
      expect(runtime.report).toHaveBeenCalledOnce();
    });
    expect(runtime.report.mock.calls[0]?.[0]).toEqual(expect.any(String));
    expect(runtime.report.mock.calls[0]?.[1]).toMatchObject({
      evidence: {
        endToEndEncrypted: true,
        matrixEventId: '$message',
        reporterSubmittedExcerpt: 'Waiting for content approval',
      },
    });
    expect(runtime.issueReadTicket).not.toHaveBeenCalled();
    expect(runtime.download).not.toHaveBeenCalled();
  });
});

function dependencies(text: string) {
  const issueReadTicket = vi.fn<ContentGateway['issueReadTicket']>(() =>
    Promise.resolve(ok({ expiresAtUnixMs: 1_800_000_000_000, ticket: 'short-lived-ticket' })),
  );
  const download = vi.fn<ContentGateway['download']>(() =>
    Promise.resolve(
      ok({
        bytes: new TextEncoder().encode(text),
        contentDigest: 'sha-256=:q6urq6urq6urq6urq6urq6urq6urq6urq6urq6urq6s=:',
        contentLength: String(new TextEncoder().encode(text).byteLength),
        mediaType: 'text/markdown',
      }),
    ),
  );
  const verify = vi.fn<ContentVerifier['verify']>((downloaded) =>
    Promise.resolve(
      ok({
        bytes: downloaded.bytes,
        digestSha256: 'ab'.repeat(32),
        mediaType: 'text/markdown',
        mode: 'text',
        text,
      }),
    ),
  );
  const approve = vi.fn<HandoffGateway['approve']>((request) =>
    Promise.resolve(ok(acceptedHandoffFixture(request))),
  );
  const listTargets = vi.fn<HandoffGateway['listTargets']>(() =>
    Promise.resolve(ok([handoffTargetFixture])),
  );
  const reconcile = vi.fn<HandoffGateway['reconcile']>((handoffId) =>
    Promise.resolve(ok(handoffSnapshotFixture(handoffId, 'delivered'))),
  );
  const revoke = vi.fn<HandoffGateway['revoke']>((handoffId) =>
    Promise.resolve(ok(handoffSnapshotFixture(handoffId, 'revoked'))),
  );
  const report = vi.fn<ModerationGateway['report']>((caseId, input) =>
    Promise.resolve(
      ok({
        caseId,
        createdAtUnixMs: 1_800_000_000_000,
        description: input.description,
        evidence: {
          endToEndEncrypted: input.evidence.endToEndEncrypted,
          matrixEventId: input.evidence.matrixEventId ?? null,
          reporterSubmittedExcerpt: input.evidence.reporterSubmittedExcerpt ?? null,
          roomCatalogId: input.evidence.roomCatalogId ?? null,
        },
        reason: input.reason,
        resolvedAtUnixMs: null,
        state: 'open',
        targetKind: input.targetKind,
        targetReference: input.targetReference,
      }),
    ),
  );
  const translate = vi.fn<MachineTranslationGateway['translate']>((request) =>
    Promise.resolve(
      ok({
        ...request,
        provenance: 'machine' as const,
        translatedText: 'Translated locally',
      }),
    ),
  );
  const moderation = {
    applyAction: async () =>
      Promise.resolve().then(() => {
        throw new Error('not used');
      }),
    inspectCapabilities: () => Promise.resolve(ok({ canModerateRoom: false, canReadAudit: false })),
    listActions: () => Promise.resolve(ok([])),
    listAudit: () => Promise.resolve(ok([])),
    listCases: () => Promise.resolve(ok([])),
    listRoomCases: () => Promise.resolve(ok([])),
    report,
    reverseAction: async () =>
      Promise.resolve().then(() => {
        throw new Error('not used');
      }),
  } satisfies ModerationGateway;
  return {
    approve,
    content: { download, issueReadTicket } satisfies ContentGateway,
    download,
    handoffs: { approve, listTargets, reconcile, revoke } satisfies HandoffGateway,
    issueReadTicket,
    listTargets,
    moderation,
    onClose: vi.fn(),
    report,
    translate,
    translation: { translate } satisfies MachineTranslationGateway,
    verifier: { verify } satisfies ContentVerifier,
    verify,
  };
}

function renderInspector(
  runtime: ReturnType<typeof dependencies>,
  selectedMessage: RoomMessageSignal = message(),
) {
  return render(
    <I18nextProvider i18n={i18n}>
      <QueryClientProvider client={new QueryClient()}>
        <ContentInspector
          catalogId="01990d9e-8400-7000-8000-000000000401"
          contentGateway={runtime.content}
          contentVerifier={runtime.verifier}
          handoffGateway={runtime.handoffs}
          message={selectedMessage}
          moderationGateway={runtime.moderation}
          onClose={runtime.onClose}
          translationGateway={runtime.translation}
        />
      </QueryClientProvider>
    </I18nextProvider>,
  );
}

function message(): RoomMessageSignal {
  return {
    actor: {
      agentId: '01990d9e-8400-7000-8000-000000000001',
      displayName: 'Build Agent',
      instanceId: '01990d9e-8400-7000-8000-000000000002',
      kind: 'agent',
      matrixUserId: '@build-agent:agent-room.test',
      provenance: 'human_confirmed_agent',
    },
    content: {
      contentId: '01990d9e-8400-7000-8000-000000000006',
      digestSha256: 'ab'.repeat(32),
      mediaType: 'text/markdown',
      sizeBytes: 128,
    },
    edited: false,
    endToEndEncrypted: true,
    lifecycle: 'active',
    matrixEventId: '$message',
    messageId: '01990d9e-8400-7000-8000-000000000003',
    preview: {
      contentType: 'text/markdown',
      language: 'zh-CN',
      riskFlags: ['untrusted_instructions'],
      sensitivity: 'normal',
      summary: 'Waiting for content approval',
      title: 'Protocol generation complete',
    },
    roomId: '!public:agent-room.test',
    serverTimestamp: 1_700_000_000_000,
    signatureStatus: 'matrix_sender_matched',
  };
}
