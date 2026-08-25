// @vitest-environment jsdom

import '@testing-library/jest-dom/vitest';

import { QueryClient, QueryClientProvider } from '@tanstack/react-query';
import { cleanup, render, screen } from '@testing-library/react';
import userEvent from '@testing-library/user-event';
import { I18nextProvider } from 'react-i18next';
import { afterEach, beforeAll, describe, expect, it, vi } from 'vitest';

import type {
  MatrixSecurityGateway,
  MatrixVerificationSession,
} from '@/features/security/domain/matrix-security';
import { MatrixVerificationInboxView } from '@/features/security/ui/matrix-verification-inbox';
import { i18n, initializeI18n } from '@/shared/i18n/i18n';
import { err, ok } from '@/shared/result';

beforeAll(async () => {
  await initializeI18n(window.localStorage, ['en']);
});

afterEach(cleanup);

describe('MatrixVerificationInboxView', () => {
  it('在任意页面显式接受入站请求后才打开 SAS 核对', async () => {
    const user = userEvent.setup();
    const session = verificationSession();
    const acceptIncomingVerification = vi.fn(async () => ok(session));
    const gateway = gatewayWithIncomingRequest({ acceptIncomingVerification });

    renderInbox(gateway);

    expect(await screen.findByText('Verify another signed-in device')).toBeVisible();
    expect(screen.getByText(/ALICE-LAPTOP/u)).toBeVisible();
    await user.click(screen.getByRole('button', { name: 'Review codes' }));

    expect(acceptIncomingVerification).toHaveBeenCalledWith('incoming-verification');
    expect(await screen.findByRole('dialog', { name: 'Verify a Matrix device' })).toBeVisible();
    expect(screen.getByText('🐶')).toBeVisible();
  });
});

function renderInbox(gateway: MatrixSecurityGateway) {
  const queryClient = new QueryClient({
    defaultOptions: { mutations: { retry: false }, queries: { retry: false } },
  });
  return render(
    <I18nextProvider i18n={i18n}>
      <QueryClientProvider client={queryClient}>
        <MatrixVerificationInboxView gateway={gateway} />
      </QueryClientProvider>
    </I18nextProvider>,
  );
}

function gatewayWithIncomingRequest(
  overrides: Partial<MatrixSecurityGateway>,
): MatrixSecurityGateway {
  const incoming = Object.freeze({
    requestId: 'incoming-verification',
    sourceDeviceId: 'ALICE-LAPTOP',
    sourceUserId: '@alice:agent-room.test',
  });
  return {
    acceptIncomingVerification: async () =>
      err({ code: 'security.verification_unavailable', retryable: false }),
    beginVerification: async () =>
      err({ code: 'security.verification_unavailable', retryable: false }),
    declineIncomingVerification: async () => ok(undefined),
    establishIdentity: async () => ok(undefined),
    getIncomingVerification: () => incoming,
    inspect: async () => err({ code: 'security.inspection_failed', retryable: true }),
    recover: async () => err({ code: 'security.recovery_failed', retryable: true }),
    setupRecovery: async () => err({ code: 'security.recovery_setup_failed', retryable: true }),
    subscribe: () => () => undefined,
    ...overrides,
  };
}

function verificationSession(): MatrixVerificationSession {
  const snapshot = Object.freeze({
    sas: Object.freeze({
      decimals: Object.freeze([1024, 2048, 4096] as const),
      emojis: Object.freeze([Object.freeze({ label: 'Dog', symbol: '🐶' })]),
    }),
    stage: 'comparing' as const,
  });
  return {
    cancel: async () => ok(undefined),
    confirm: async () => ok(undefined),
    dispose: vi.fn(),
    getSnapshot: () => snapshot,
    mismatch: vi.fn(),
    subscribe: () => () => undefined,
  };
}
