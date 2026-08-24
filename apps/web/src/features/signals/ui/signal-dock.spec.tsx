// @vitest-environment jsdom

import '@testing-library/jest-dom/vitest';

import { cleanup, render, screen, within } from '@testing-library/react';
import userEvent from '@testing-library/user-event';
import { I18nextProvider } from 'react-i18next';
import { afterEach, beforeAll, describe, expect, it, vi } from 'vitest';

import type { SignalAction, SignalKind, SignalProjection } from '@/features/signals/domain/signal';
import { SignalDock } from '@/features/signals/ui/signal-dock';
import { i18n, initializeI18n } from '@/shared/i18n/i18n';

beforeAll(async () => {
  await initializeI18n(window.localStorage, ['en']);
});

afterEach(cleanup);

describe('SignalDock', () => {
  it('默认保持单行，只把显式点击转换为领域动作', async () => {
    const user = userEvent.setup();
    const onAction = vi.fn<(action: SignalAction) => void>();
    renderDock({
      onAction,
      signals: [signal('room_message', 'Room update'), signal('mention', 'Review requested')],
    });

    expect(screen.queryByRole('toolbar', { name: 'Signal controls' })).not.toBeInTheDocument();
    await user.click(screen.getByRole('button', { name: 'Review requested' }));

    expect(onAction).toHaveBeenCalledWith({ kind: 'open_message', messageId: 'mention' });
  });

  it('展开后按类型过滤，并可冻结一个不可变观察快照', async () => {
    const user = userEvent.setup();
    const view = renderDock({
      signals: [signal('room_message', 'Initial room message'), signal('mention', 'Mention')],
    });
    await user.click(screen.getByRole('button', { name: 'Expand signal timeline' }));
    await user.click(screen.getByRole('button', { name: 'Room messages' }));

    const timeline = screen.getByRole('list', { name: 'Room signal timeline' });
    expect(within(timeline).getByText('Initial room message')).toBeInTheDocument();
    expect(within(timeline).queryByText('Mention')).not.toBeInTheDocument();

    await user.click(screen.getByRole('button', { name: 'Freeze' }));
    view.rerender(element([signal('room_message', 'New room message')]));
    expect(screen.getByText('Initial room message')).toBeInTheDocument();
    expect(screen.queryByText('New room message')).not.toBeInTheDocument();

    await user.click(screen.getByRole('button', { name: 'Resume' }));
    expect(within(timeline).getByText('New room message')).toBeInTheDocument();
  });

  it('六类信号使用同一个投影表，不靠组件条件链拼接', async () => {
    const user = userEvent.setup();
    renderDock({ signals: allKinds.map((kind) => signal(kind, kind)) });

    await user.click(screen.getByRole('button', { name: 'Expand signal timeline' }));

    for (const label of [
      'Room messages',
      'Direct messages',
      'Mentions',
      'Task references',
      'Pending handoffs',
      'Sync issues',
    ]) {
      expect(screen.getByRole('button', { name: label })).toBeInTheDocument();
    }
  });
});

const allKinds: readonly SignalKind[] = [
  'room_message',
  'direct_message',
  'mention',
  'task_reference',
  'handoff_pending',
  'sync_issue',
];

function renderDock({
  onAction = vi.fn(),
  signals,
}: {
  readonly onAction?: (action: SignalAction) => void;
  readonly signals: readonly SignalProjection[];
}) {
  return render(element(signals, onAction));
}

function element(
  signals: readonly SignalProjection[],
  onAction: (action: SignalAction) => void = vi.fn(),
) {
  return (
    <I18nextProvider i18n={i18n}>
      <SignalDock
        onAction={onAction}
        onRetry={vi.fn()}
        selectedSignalId={null}
        signals={signals}
        state="ready"
      />
    </I18nextProvider>
  );
}

function signal(kind: SignalKind, title: string): SignalProjection {
  return {
    action: { kind: 'open_message', messageId: kind },
    actor:
      kind === 'sync_issue'
        ? null
        : {
            agentId: '01990d9e-8400-7000-8000-000000000001',
            displayName: 'Build Agent',
            instanceId: '01990d9e-8400-7000-8000-000000000002',
            matrixUserId: '@build-agent:agent-room.test',
            provenance: 'human_confirmed_agent',
          },
    edited: false,
    kind,
    lifecycle: 'active',
    occurredAtUnixMs: 1_700_000_000_000,
    riskFlags: [],
    signalId: kind,
    summary: 'Signal summary',
    title,
  };
}
