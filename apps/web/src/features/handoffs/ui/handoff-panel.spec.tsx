// @vitest-environment jsdom

import '@testing-library/jest-dom/vitest';

import { cleanup, render, screen, waitFor } from '@testing-library/react';
import userEvent from '@testing-library/user-event';
import { I18nextProvider } from 'react-i18next';
import { afterEach, beforeAll, describe, expect, it, vi } from 'vitest';

import type { HandoffGateway, HandoffTarget } from '@/features/handoffs/domain/handoff';
import {
  acceptedHandoffFixture,
  handoffSnapshotFixture,
  handoffTargetFixture,
} from '@/features/handoffs/testing/handoff-fixtures';
import { HandoffPanel } from '@/features/handoffs/ui/handoff-panel';
import type { RoomMessageSignal } from '@/features/messages/domain/message';
import { i18n, initializeI18n } from '@/shared/i18n/i18n';
import { ok } from '@/shared/result';

const handoffId = '01990d9e-8400-7000-8000-000000000090';
const offlineTarget: HandoffTarget = Object.freeze({
  ...handoffTargetFixture,
  adapterType: 'claude-desktop',
  agentDisplayName: 'Research Agent',
  agentId: '01990d9e-8400-7000-8000-000000000021',
  device: Object.freeze({
    deviceId: '01990d9e-8400-7000-8000-000000000023',
    label: 'Travel PC',
    platform: 'windows',
  }),
  instanceId: '01990d9e-8400-7000-8000-000000000022',
  instanceStatus: 'offline',
  leaseExpiresAtUnixMs: null,
  online: false,
});

beforeAll(async () => {
  await initializeI18n(window.localStorage, ['en']);
});

afterEach(cleanup);

describe('HandoffPanel', () => {
  it('按 Agent 与设备展示账号目标，并允许明确选择离线队列', async () => {
    const user = userEvent.setup();
    const runtime = gateway();
    render(
      <I18nextProvider i18n={i18n}>
        <HandoffPanel
          gateway={runtime.value}
          handoffIds={{ next: () => handoffId }}
          message={message()}
          now={() => 1_700_000_000_000}
          onBack={vi.fn()}
        />
      </I18nextProvider>,
    );

    expect(await screen.findByText('Local Codex Agent')).toBeInTheDocument();
    expect(screen.getByText('Research Agent')).toBeInTheDocument();
    expect(screen.getByText('Studio PC')).toBeInTheDocument();
    expect(screen.getByText('Travel PC')).toBeInTheDocument();
    expect(screen.getByText('Online · deliver now')).toBeInTheDocument();
    expect(screen.getByText('Offline · queue')).toBeInTheDocument();

    await user.click(screen.getByRole('radio', { name: /claude-desktop/i }));
    expect(screen.getByText('Queue until this instance reconnects')).toBeInTheDocument();
    await user.click(screen.getByRole('button', { name: 'Confirm handoff' }));

    await screen.findByText('Queued for one instance');
    expect(runtime.approve).toHaveBeenCalledOnce();
    expect(runtime.approve.mock.calls[0]?.[0].target.instanceId).toBe(offlineTarget.instanceId);
  });
});

function gateway() {
  const approve = vi.fn<HandoffGateway['approve']>((request) =>
    Promise.resolve(ok(acceptedHandoffFixture(request))),
  );
  const value: HandoffGateway = {
    approve,
    listTargets: async () => ok([handoffTargetFixture, offlineTarget]),
    reconcile: async (requestedHandoffId) =>
      ok(handoffSnapshotFixture(requestedHandoffId, 'delivered')),
    revoke: async (requestedHandoffId) => ok(handoffSnapshotFixture(requestedHandoffId, 'revoked')),
  };
  return { approve, value };
}

function message(): RoomMessageSignal & {
  readonly content: NonNullable<RoomMessageSignal['content']>;
  readonly preview: NonNullable<RoomMessageSignal['preview']>;
} {
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
    matrixEventId: '$message:agent-room.test',
    messageId: '01990d9e-8400-7000-8000-000000000003',
    preview: {
      contentType: 'text/markdown',
      language: 'en',
      riskFlags: ['untrusted_instructions'],
      sensitivity: 'normal',
      summary: 'Verified context',
      title: 'Protocol generation complete',
    },
    roomId: '!public:agent-room.test',
    serverTimestamp: 1_700_000_000_000,
    signatureStatus: 'matrix_sender_matched',
  };
}
