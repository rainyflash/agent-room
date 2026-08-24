// @vitest-environment jsdom

import '@testing-library/jest-dom/vitest';

import { cleanup, render, screen, waitFor } from '@testing-library/react';
import userEvent from '@testing-library/user-event';
import { I18nextProvider } from 'react-i18next';
import { afterEach, beforeAll, describe, expect, it, vi } from 'vitest';

import type { ContentGateway, ContentVerifier } from '@/features/messages/domain/content';
import type { RoomMessageSignal } from '@/features/messages/domain/message';
import { ContentInspector } from '@/features/messages/ui/content-inspector';
import { i18n, initializeI18n } from '@/shared/i18n/i18n';
import { ok } from '@/shared/result';

beforeAll(async () => {
  await initializeI18n(window.localStorage, ['en']);
});

afterEach(cleanup);

describe('ContentInspector', () => {
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
  return {
    content: { download, issueReadTicket } satisfies ContentGateway,
    download,
    issueReadTicket,
    onClose: vi.fn(),
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
      <ContentInspector
        contentGateway={runtime.content}
        contentVerifier={runtime.verifier}
        message={selectedMessage}
        onClose={runtime.onClose}
      />
    </I18nextProvider>,
  );
}

function message(): RoomMessageSignal {
  return {
    actor: {
      agentId: '01990d9e-8400-7000-8000-000000000001',
      displayName: 'Build Agent',
      instanceId: '01990d9e-8400-7000-8000-000000000002',
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
    lifecycle: 'active',
    matrixEventId: '$message',
    messageId: '01990d9e-8400-7000-8000-000000000003',
    preview: {
      contentType: 'text/markdown',
      riskFlags: ['untrusted_instructions'],
      sensitivity: 'normal',
      summary: 'Waiting for content approval',
      title: 'Protocol generation complete',
    },
    roomId: '!public:agent-room.test',
    serverTimestamp: 1_700_000_000_000,
  };
}
