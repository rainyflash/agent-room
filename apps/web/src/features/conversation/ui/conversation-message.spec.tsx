// @vitest-environment jsdom

import '@testing-library/jest-dom/vitest';

import { cleanup, render, screen } from '@testing-library/react';
import { I18nextProvider } from 'react-i18next';
import { afterEach, beforeAll, beforeEach, describe, expect, it, vi } from 'vitest';

import type { RoomMessageSignal } from '@/features/messages/domain/message';
import type { AgentDelivery } from '@/features/conversation/domain/message-delivery';
import { ConversationMessage } from '@/features/conversation/ui/conversation-message';
import { i18n, initializeI18n } from '@/shared/i18n/i18n';

const message: RoomMessageSignal = {
  actor: {
    displayName: 'Rainy',
    kind: 'human',
    matrixUserId: '@rainy:agent-room.test',
    principalId: '01990d9e-8400-7000-8000-000000000003',
    provenance: 'human',
  },
  content: null,
  edited: false,
  endToEndEncrypted: false,
  lifecycle: 'active',
  matrixEventId: '$mine',
  messageId: '01990d9e-8400-7000-8000-000000000004',
  preview: {
    contentType: 'text/plain',
    conversation: { mentions: [], text: 'Please review the release notes.' },
    riskFlags: [],
    sensitivity: 'normal',
    summary: 'Please review the release notes.',
    title: 'Please review the release notes.',
  },
  roomId: '!chat:agent-room.test',
  serverTimestamp: 1_000,
  signatureStatus: 'matrix_sender_matched',
};

beforeAll(async () => {
  await initializeI18n(window.localStorage, ['en']);
});

beforeEach(async () => {
  await i18n.changeLanguage('en');
});

afterEach(cleanup);

function renderMine(delivery: readonly AgentDelivery[]) {
  render(
    <I18nextProvider i18n={i18n}>
      <ConversationMessage
        delivery={delivery}
        editable
        message={message}
        names={new Map()}
        onReply={vi.fn()}
        own
        parent={undefined}
        time={new Intl.DateTimeFormat('en', { timeStyle: 'short' })}
      />
    </I18nextProvider>,
  );
  return screen.getByText(/Sent|agent|Ada/u, { selector: 'summary span' }).closest('details');
}

describe('自己消息下的送达标记', () => {
  it('没有 Agent 回执时只是一个“已发送”的小标记，点开才说还没人确认', () => {
    const mark = renderMine([]);

    expect(mark).not.toHaveAttribute('open');
    expect(screen.getByText('Sent', { selector: 'summary span' })).toBeInTheDocument();
    expect(mark).toHaveTextContent('Sent to room');
    expect(mark).toHaveTextContent('No agent has confirmed it yet');
  });

  it('一个 Agent 时直接写它的进度', () => {
    renderMine([{ agentId: 'a', name: 'Ada', stage: 'replied' }]);

    expect(screen.getByText('Ada · Replied', { selector: 'summary span' })).toBeInTheDocument();
  });

  it('几个 Agent 时写个数，点开列出每个；有回复需要处理时标记换成提醒色', () => {
    const mark = renderMine([
      { agentId: 'a', name: 'Ada', stage: 'replied' },
      { agentId: 'b', name: 'Scout', stage: 'needs_review' },
    ]);

    expect(screen.getByText('2 agents', { selector: 'summary span' })).toBeInTheDocument();
    expect(mark).toHaveAttribute('data-attention', 'true');
    expect(mark).toHaveTextContent('Ada · Replied');
    expect(mark).toHaveTextContent('Scout · Reply needs attention');
  });
});
