// @vitest-environment jsdom
import '@testing-library/jest-dom/vitest';
import { cleanup, render, screen, waitFor } from '@testing-library/react';
import userEvent from '@testing-library/user-event';
import { I18nextProvider } from 'react-i18next';
import { afterEach, beforeAll, describe, expect, it, vi } from 'vitest';
import { ConversationPanel } from './conversation-panel';
import type {
  MessagePublicationRequest,
  MessagePublisher,
} from '@/features/messages/domain/publication';
import type { RoomMessageSignal } from '@/features/messages/domain/message';
import { initializeI18n, i18n } from '@/shared/i18n/i18n';
import { ok } from '@/shared/result';

const roomId = '!chat:agent-room.test';
const submissionId = '01990d9e-8400-7000-8000-000000000003';
const agentId = '@assistant:agent-room.test';
beforeAll(async () => {
  await initializeI18n(window.localStorage, ['en']);
});
afterEach(cleanup);

function harness(unknown = false, messages: readonly RoomMessageSignal[] = []) {
  const publish = vi.fn((request: MessagePublicationRequest) =>
    Promise.resolve(
      ok(
        unknown
          ? {
              kind: 'pending_reconciliation' as const,
              submissionId: request.submissionId,
              transactionId: `agent-room-message-${request.submissionId}`,
            }
          : {
              kind: 'published' as const,
              matrixEventId: '$sent',
              reused: false,
              submissionId: request.submissionId,
            },
      ),
    ),
  );
  const reconcile = vi.fn((id: string) =>
    Promise.resolve(
      ok({ kind: 'published' as const, matrixEventId: '$sent', reused: true, submissionId: id }),
    ),
  );
  const publisher: MessagePublisher = {
    publish,
    reconcile,
    resolveIdentity: () =>
      Promise.resolve(
        ok({
          kind: 'human',
          displayName: 'Rainy',
          matrixUserId: '@rainy:agent-room.test',
          principalId: submissionId,
          source: 'matrix_human_session',
        }),
      ),
  };
  render(
    <I18nextProvider i18n={i18n}>
      <ConversationPanel
        messages={messages}
        publisher={publisher}
        roomId={roomId}
        roomName="Lobby"
        state="ready"
        participants={[{ matrixUserId: agentId, displayName: 'Ada' }]}
        submissionIds={{ next: () => submissionId }}
      />
    </I18nextProvider>,
  );
  return { publish, reconcile };
}

describe('人与 Agent 直接聊天', () => {
  it('一段输入与稳定身份提及直接发布，并清空已发送草稿', async () => {
    const runtime = harness();
    const user = userEvent.setup();
    await waitFor(() => {
      expect(screen.getByRole('textbox', { name: 'Message' })).toBeEnabled();
    });
    await user.selectOptions(screen.getByRole('combobox', { name: 'Mention an agent' }), agentId);
    await user.type(screen.getByRole('textbox', { name: 'Message' }), 'What do you think?');
    await user.click(screen.getByRole('button', { name: 'Send' }));
    await waitFor(() => {
      expect(runtime.publish).toHaveBeenCalledOnce();
    });
    expect(runtime.publish.mock.calls[0]?.[0]).toMatchObject({
      roomId,
      submissionId,
      mediaType: 'text/plain',
      conversation: { text: 'What do you think?', mentions: [agentId] },
    });
    await waitFor(() => {
      expect(screen.getByRole('textbox', { name: 'Message' })).toHaveValue('');
    });
  });

  it('结果未知时锁住新发送，只使用原提交查询', async () => {
    const runtime = harness(true);
    const user = userEvent.setup();
    await waitFor(() => {
      expect(screen.getByRole('textbox', { name: 'Message' })).toBeEnabled();
    });
    await user.type(screen.getByRole('textbox', { name: 'Message' }), 'hello');
    await user.click(screen.getByRole('button', { name: 'Send' }));
    await screen.findByText('Delivery is being confirmed. Check before sending again.');
    expect(screen.getByRole('button', { name: 'Send' })).toBeDisabled();
    await user.click(screen.getByRole('button', { name: 'Check delivery' }));
    await waitFor(() => {
      expect(runtime.reconcile).toHaveBeenCalledWith(submissionId);
    });
    expect(runtime.publish).toHaveBeenCalledOnce();
  });

  it('回复保留关联且远端 HTML 只显示为文字', async () => {
    const message: RoomMessageSignal = {
      actor: {
        agentId: submissionId,
        instanceId: submissionId,
        displayName: 'Ada',
        kind: 'agent',
        matrixUserId: agentId,
        provenance: 'human_confirmed_agent',
      },
      messageId: submissionId,
      matrixEventId: '$question',
      roomId,
      lifecycle: 'active',
      edited: false,
      endToEndEncrypted: false,
      serverTimestamp: 1_000,
      signatureStatus: 'instance_verified',
      content: null,
      preview: {
        title: 'Question',
        summary: 'Question',
        contentType: 'text/plain',
        riskFlags: [],
        sensitivity: 'normal',
        conversation: { text: '<img src=x onerror=alert(1)>', mentions: [] },
      },
    };
    const runtime = harness(false, [message]);
    const user = userEvent.setup();
    expect(screen.getByText('<img src=x onerror=alert(1)>')).toBeInTheDocument();
    expect(document.querySelector('img')).toBeNull();
    await waitFor(() => {
      expect(screen.getByRole('button', { name: 'Reply to Ada' })).toBeEnabled();
    });
    await user.click(screen.getByRole('button', { name: 'Reply to Ada' }));
    await user.type(screen.getByRole('textbox', { name: 'Message' }), 'My answer');
    await user.click(screen.getByRole('button', { name: 'Send' }));
    await waitFor(() => {
      expect(runtime.publish).toHaveBeenCalledOnce();
    });
    expect(runtime.publish.mock.calls[0]?.[0]).toMatchObject({
      relation: { kind: 'reply', targetMessageId: submissionId },
      conversation: { mentions: [agentId] },
    });
  });
});
