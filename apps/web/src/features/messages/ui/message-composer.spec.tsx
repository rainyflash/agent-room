// @vitest-environment jsdom

import '@testing-library/jest-dom/vitest';

import { cleanup, render, screen, waitFor } from '@testing-library/react';
import userEvent from '@testing-library/user-event';
import { I18nextProvider } from 'react-i18next';
import { afterEach, beforeAll, describe, expect, it, vi } from 'vitest';

import type { MessageSubmissionIdFactory } from '@/features/messages/adapters/browser-submission-id-factory';
import type {
  MessagePublicationResult,
  MessagePublisher,
  MessagePublisherIdentity,
} from '@/features/messages/domain/publication';
import { MessageComposer } from '@/features/messages/ui/message-composer';
import {
  RuntimeCompatibilityBoundary,
  type RuntimeCompatibility,
} from '@/features/updates/ui/runtime-compatibility-context';
import { i18n, initializeI18n } from '@/shared/i18n/i18n';
import { err, ok } from '@/shared/result';

const submissionId = '01990d9e-8400-7000-8000-000000000003';
const identity: MessagePublisherIdentity = {
  agentId: '01990d9e-8400-7000-8000-000000000001',
  displayName: 'Build Agent',
  instanceId: '01990d9e-8400-7000-8000-000000000002',
  matrixUserId: '@build-agent:agent-room.test',
  provenance: 'human_confirmed_agent',
  source: 'bridge_agent_instance',
};
const unknown: MessagePublicationResult = ok({
  kind: 'pending_reconciliation',
  submissionId,
  transactionId: `agent-room-message-${submissionId}`,
});
const published: MessagePublicationResult = ok({
  kind: 'published',
  matrixEventId: '$accepted',
  reused: false,
  submissionId,
});
const submissionIds: MessageSubmissionIdFactory = { next: () => submissionId };

beforeAll(async () => {
  await initializeI18n(window.localStorage, ['en']);
});

afterEach(cleanup);

describe('MessageComposer', () => {
  it('折叠时不探测身份、不上传正文也不查询 Matrix', async () => {
    const runtime = publisher({
      identityResult: err({ code: 'publication.bridge_unavailable', retryable: false }),
    });
    const user = userEvent.setup();
    renderComposer(runtime.value);

    expect(runtime.resolveIdentity).not.toHaveBeenCalled();
    expect(runtime.publish).not.toHaveBeenCalled();
    expect(runtime.reconcile).not.toHaveBeenCalled();

    await user.click(screen.getByRole('button', { name: 'Open the message composer' }));

    expect(await screen.findByText('No trusted sending identity is available')).toBeInTheDocument();
    expect(runtime.resolveIdentity).toHaveBeenCalledOnce();
    expect(runtime.publish).not.toHaveBeenCalled();
  });

  it('展示身份、目标和发出风险，并用一个 UUIDv7 意图提交', async () => {
    const runtime = publisher({ publishResults: [published] });
    const user = userEvent.setup();
    renderComposer(runtime.value);
    await user.click(screen.getByRole('button', { name: 'Open the message composer' }));

    expect(await screen.findByText('Build Agent')).toBeInTheDocument();
    expect(screen.getByText('Builders Exchange')).toBeInTheDocument();
    expect(screen.getByText('Local Bridge · Agent instance key')).toBeInTheDocument();

    await user.type(screen.getByLabelText('Preview title'), 'Protocol review');
    await user.type(screen.getByLabelText('Preview summary'), 'Please review the protocol change.');
    await user.type(
      screen.getByLabelText('Full content'),
      'Review https://example.com and keep <script> inert.',
    );
    expect(screen.getByText(/External links detected/u)).toBeInTheDocument();
    expect(screen.getByText(/HTML markup will remain inert/u)).toBeInTheDocument();

    await user.click(screen.getByRole('button', { name: 'Sign and send' }));
    expect(await screen.findByText('Message accepted')).toBeInTheDocument();

    expect(runtime.publish).toHaveBeenCalledOnce();
    expect(runtime.publish.mock.calls[0]?.[0]).toMatchObject({
      riskFlags: ['external_links', 'html_markup'],
      roomId: '!builders:agent-room.test',
      submissionId,
      title: 'Protocol review',
    });
  });

  it('未知状态可最小化恢复，但永远不暴露重发动作', async () => {
    const runtime = publisher({ publishResults: [unknown], reconcileResults: [published] });
    const user = userEvent.setup();
    renderComposer(runtime.value);
    await user.click(screen.getByRole('button', { name: 'Open the message composer' }));
    await screen.findByText('Build Agent');
    await user.type(screen.getByLabelText('Preview title'), 'Unknown commit');
    await user.type(screen.getByLabelText('Preview summary'), 'A transport interruption test.');
    await user.type(screen.getByLabelText('Full content'), 'Stable content');
    await user.click(screen.getByRole('button', { name: 'Sign and send' }));

    expect(await screen.findByText('Submission status unknown')).toBeInTheDocument();
    expect(screen.queryByRole('button', { name: 'Retry same submission' })).not.toBeInTheDocument();
    expect(runtime.publish).toHaveBeenCalledOnce();

    await user.click(
      screen.getByRole('button', { name: 'Minimize without losing submission state' }),
    );
    expect(
      screen.getByRole('button', { name: 'Resume the current message submission' }),
    ).toBeInTheDocument();
    await user.click(screen.getByRole('button', { name: 'Resume the current message submission' }));
    await user.click(screen.getByRole('button', { name: 'Query submission status' }));

    await waitFor(() => {
      expect(screen.getByText('Message accepted')).toBeInTheDocument();
    });
    expect(runtime.publish).toHaveBeenCalledOnce();
    expect(runtime.reconcile).toHaveBeenCalledOnce();
    expect(runtime.reconcile).toHaveBeenCalledWith(submissionId);
  });

  it('旧协议等待更新时进入只读且不创建发送意图', async () => {
    const runtime = publisher({ publishResults: [published] });
    const compatibility: RuntimeCompatibility = {
      applyUpdate: () => Promise.resolve(),
      updateWaiting: true,
      writes: { allowed: false, reason: 'update_required' },
    };
    const user = userEvent.setup();
    renderComposer(runtime.value, compatibility);
    await user.click(screen.getByRole('button', { name: 'Open the message composer' }));
    await screen.findByText('Build Agent');

    expect(screen.getByText('Read-only safety mode')).toBeInTheDocument();
    expect(screen.getByRole('button', { name: 'Sign and send' })).toBeDisabled();
    expect(runtime.publish).not.toHaveBeenCalled();
  });
});

function renderComposer(value: MessagePublisher, compatibility?: RuntimeCompatibility) {
  const composer = (
    <MessageComposer
      publisher={value}
      roomId="!builders:agent-room.test"
      roomName="Builders Exchange"
      submissionIds={submissionIds}
    />
  );
  return render(
    <I18nextProvider i18n={i18n}>
      {compatibility === undefined ? (
        composer
      ) : (
        <RuntimeCompatibilityBoundary value={compatibility}>
          {composer}
        </RuntimeCompatibilityBoundary>
      )}
    </I18nextProvider>,
  );
}

function publisher(
  options: {
    readonly identityResult?: Awaited<ReturnType<MessagePublisher['resolveIdentity']>>;
    readonly publishResults?: readonly MessagePublicationResult[];
    readonly reconcileResults?: readonly MessagePublicationResult[];
  } = {},
) {
  const resolveIdentity = vi.fn<MessagePublisher['resolveIdentity']>(() =>
    Promise.resolve(options.identityResult ?? ok(identity)),
  );
  const publish = vi.fn<MessagePublisher['publish']>();
  for (const result of options.publishResults ?? [published]) {
    publish.mockResolvedValueOnce(result);
  }
  const reconcile = vi.fn<MessagePublisher['reconcile']>();
  for (const result of options.reconcileResults ?? [unknown]) {
    reconcile.mockResolvedValueOnce(result);
  }
  const value: MessagePublisher = { publish, reconcile, resolveIdentity };
  return { publish, reconcile, resolveIdentity, value };
}
