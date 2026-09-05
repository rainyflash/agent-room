// @vitest-environment jsdom

import '@testing-library/jest-dom/vitest';

import { cleanup, render, screen } from '@testing-library/react';
import userEvent from '@testing-library/user-event';
import { I18nextProvider } from 'react-i18next';
import { afterEach, beforeAll, describe, expect, it, vi } from 'vitest';

import type { DirectSession } from '@/features/direct-sessions/domain/direct-session';
import { DirectConversationDock } from '@/features/direct-sessions/ui/direct-conversation-dock';
import type { DirectSessionController } from '@/features/direct-sessions/ui/use-direct-session-controller';
import { i18n, initializeI18n } from '@/shared/i18n/i18n';
import { err } from '@/shared/result';

vi.mock('@/features/messages/ui/message-layer', () => ({
  MessageLayer: ({ roomId }: { readonly roomId: string }) => (
    <div data-testid="direct-message-layer">{roomId}</div>
  ),
}));

const session: DirectSession = Object.freeze({
  catalogId: '01990d9e-8400-7000-8000-000000000010',
  contactPolicy: Object.freeze({
    agentBlocksPrincipal: false,
    deliveryAllowed: true,
    presenceDisclosure: 'coarse',
    principalBlocksAgent: false,
  }),
  lifecycle: 'active',
  matrixRoomId: '!direct:agent-room.test',
  roomInstanceId: '01990d9e-8400-7000-8000-000000000011',
  target: Object.freeze({
    agentId: '01990d9e-8400-7000-8000-000000000012',
    avatarContentId: null,
    displayName: 'Review Agent',
    matrixUserId: '@review-agent:agent-room.test',
  }),
  version: 0,
});

beforeAll(async () => {
  await initializeI18n(window.localStorage, ['en']);
});

afterEach(cleanup);

describe('DirectConversationDock', () => {
  it('只显示持久会话并由调用方持有 URL 选择状态', async () => {
    const user = userEvent.setup();
    const onActiveSessionChange = vi.fn();
    renderDock(controller(), null, onActiveSessionChange);

    await user.click(screen.getByRole('button', { name: 'Open conversation with Review Agent' }));

    expect(onActiveSessionChange).toHaveBeenCalledWith(session.catalogId);
    expect(screen.queryByTestId('direct-message-layer')).not.toBeInTheDocument();
  });

  it('展示粗粒度披露、复用消息层并通过协调器执行屏蔽', async () => {
    const user = userEvent.setup();
    const runtime = controller();
    renderDock(runtime, session.catalogId, vi.fn());

    expect(screen.getByText('Coarse presence only')).toBeInTheDocument();
    expect(screen.getByText('Delivery allowed')).toBeInTheDocument();
    expect(screen.getByTestId('direct-message-layer')).toHaveTextContent('!direct:agent-room.test');

    await user.click(screen.getByRole('button', { name: 'Block' }));

    expect(runtime.setBlocked).toHaveBeenCalledWith(session.target, true);
  });
});

function renderDock(
  runtime: DirectSessionController,
  activeCatalogId: string | null,
  onActiveSessionChange: (catalogId: string | null) => void,
) {
  return render(
    <I18nextProvider i18n={i18n}>
      <DirectConversationDock
        activeCatalogId={activeCatalogId}
        controller={runtime}
        onActiveSessionChange={onActiveSessionChange}
        onSelectedMessageChange={() => undefined}
        selectedMessageId={null}
      />
    </I18nextProvider>,
  );
}

function controller(): DirectSessionController {
  return {
    blocking: false,
    clearFailure: vi.fn(),
    failure: null,
    loading: false,
    markDisplayed: vi.fn(() => Promise.resolve(undefined)),
    openAgent: vi.fn(() =>
      Promise.resolve(err({ code: 'direct_session.test_unavailable', retryable: false })),
    ),
    opening: false,
    retry: vi.fn(() => Promise.resolve(undefined)),
    sessions: [session],
    setBlocked: vi.fn(() =>
      Promise.resolve(err({ code: 'direct_session.test_unavailable', retryable: false })),
    ),
  };
}
