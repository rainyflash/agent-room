// @vitest-environment jsdom

import '@testing-library/jest-dom/vitest';

import { cleanup, render, screen } from '@testing-library/react';
import userEvent from '@testing-library/user-event';
import { I18nextProvider } from 'react-i18next';
import { afterEach, beforeAll, describe, expect, it, vi } from 'vitest';

import type { RoomMessageSignal } from '@/features/messages/domain/message';
import { MessageSignalDock } from '@/features/messages/ui/message-signal-dock';
import { i18n, initializeI18n } from '@/shared/i18n/i18n';

beforeAll(async () => {
  await initializeI18n(window.localStorage, ['en']);
});

afterEach(cleanup);

describe('MessageSignalDock', () => {
  it('只展示预览元数据并把显式选择交给 URL 胶水层', async () => {
    const user = userEvent.setup();
    const onSelectMessage = vi.fn();
    renderDock({ messages: [message()], onSelectMessage });

    const signal = screen.getByRole('button', { name: /Protocol generation complete/u });
    expect(signal).toHaveTextContent('Waiting for content approval');
    expect(signal).toHaveAttribute('aria-pressed', 'false');

    await user.click(signal);

    expect(onSelectMessage).toHaveBeenCalledWith('01990d9e-8400-7000-8000-000000000003');
  });

  it('撤回消息不保留旧摘要或伪造正文入口', () => {
    renderDock({
      messages: [{ ...message(), content: null, lifecycle: 'redacted', preview: null }],
      onSelectMessage: vi.fn(),
    });

    expect(screen.getByRole('button', { name: /Message withdrawn/u })).toHaveTextContent(
      'original preview and content reference are unavailable',
    );
  });
});

function renderDock({
  messages,
  onSelectMessage,
}: {
  readonly messages: readonly RoomMessageSignal[];
  readonly onSelectMessage: (messageId: string) => void;
}) {
  return render(
    <I18nextProvider i18n={i18n}>
      <MessageSignalDock
        messages={messages}
        onRetry={vi.fn()}
        onSelectMessage={onSelectMessage}
        selectedMessageId={null}
        state="ready"
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
