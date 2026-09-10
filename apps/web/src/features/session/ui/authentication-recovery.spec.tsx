// @vitest-environment jsdom
import '@testing-library/jest-dom/vitest';
import { cleanup, render, screen } from '@testing-library/react';
import userEvent from '@testing-library/user-event';
import { I18nextProvider } from 'react-i18next';
import { afterEach, beforeAll, describe, expect, it, vi } from 'vitest';
import { AuthenticationRecovery } from './authentication-recovery';
import { saveAutomationGrantDraft } from '@/features/automation/adapters/automation-grant-draft';
import type { ControlPlaneGateway, WebSession } from '@/features/session/domain/session';
import { i18n, initializeI18n } from '@/shared/i18n/i18n';
import { err, ok } from '@/shared/result';

beforeAll(async () => {
  await initializeI18n(window.localStorage, ['en']);
});
afterEach(() => {
  cleanup();
  window.sessionStorage.clear();
});

const principal: WebSession = {
  principalId: '0198b601-77a1-7bb8-83eb-a8fe68c97e42',
  matrixUserId: '@human:example.test',
  displayName: 'Human',
  locale: 'en',
  authenticatedAtUnixMs: 0,
  expiresAtUnixMs: 2_000_000_000_000,
  recentlyAuthenticated: false,
};

describe('AuthenticationRecovery', () => {
  it('旧登录仍有效时明确显示验证失败，并通过新验证请求回到保存的房间', async () => {
    window.history.replaceState({}, '', '/lobby/room/instance/matrix?view=conversation');
    const returnPath = `${window.location.pathname}${window.location.search}`;
    expect(
      saveAutomationGrantDraft(principal.principalId, {
        agentId: '0198b601-77a1-7bb8-83eb-a8fe68c97e44',
        agentInstanceId: '0198b601-77a1-7bb8-83eb-a8fe68c97e45',
        roomCatalogId: '0198b601-77a1-7bb8-83eb-a8fe68c97e46',
        audience: 'any_room_member',
        lifetimeSeconds: 3600,
        maxMessagesPerMinute: 3,
        maxTotalMessages: 20,
        messageKinds: ['reply'],
        requiresRiskScan: true,
      }).ok,
    ).toBe(true);
    const beginAuthentication = vi
      .fn<ControlPlaneGateway['beginAuthentication']>()
      .mockResolvedValue(ok({ kind: 'browser-navigation' }));
    const logout = vi.fn<ControlPlaneGateway['logout']>();
    const gateway: ControlPlaneGateway = {
      beginAuthentication,
      readSession: vi.fn(),
      logout,
    };
    render(
      <I18nextProvider i18n={i18n}>
        <AuthenticationRecovery gateway={gateway} principal={principal} reason="expired" />
      </I18nextProvider>,
    );
    expect(screen.getByText(/permission change has not been approved/u)).toBeInTheDocument();
    expect(beginAuthentication).not.toHaveBeenCalled();
    await userEvent.click(screen.getByRole('button', { name: 'Continue verification' }));
    expect(beginAuthentication).toHaveBeenCalledExactlyOnceWith(returnPath);
    expect(logout).not.toHaveBeenCalled();
  });

  it('无法打开验证页时显示可重试错误', async () => {
    const beginAuthentication = vi
      .fn<ControlPlaneGateway['beginAuthentication']>()
      .mockResolvedValue(
        err({
          boundary: 'browser',
          code: 'browser.authentication_navigation_failed',
          offline: false,
          retryable: true,
        }),
      );
    render(
      <I18nextProvider i18n={i18n}>
        <AuthenticationRecovery
          gateway={{ beginAuthentication, readSession: vi.fn(), logout: vi.fn() }}
          principal={null}
          reason="failed"
        />
      </I18nextProvider>,
    );
    await userEvent.click(screen.getByRole('button', { name: 'Continue verification' }));
    expect(
      await screen.findByText('Verification could not be opened. Please retry.'),
    ).toBeInTheDocument();
    expect(screen.getByRole('button', { name: 'Continue verification' })).toBeEnabled();
  });
});
