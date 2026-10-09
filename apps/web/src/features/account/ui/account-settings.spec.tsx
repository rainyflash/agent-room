// @vitest-environment jsdom

import '@testing-library/jest-dom/vitest';

import { cleanup, fireEvent, render, screen, waitFor } from '@testing-library/react';
import type { ReactNode } from 'react';
import { I18nextProvider } from 'react-i18next';
import { afterEach, beforeAll, beforeEach, describe, expect, it, vi } from 'vitest';

import { useAppServices } from '@/app/app-services';
import type { AccountGateway } from '@/features/account/domain/account';
import { AccountDeletedNotice } from '@/features/account/ui/account-deleted-notice';
import { AccountSettings } from '@/features/account/ui/account-settings';
import { useOptionalSession } from '@/features/session/ui/session-provider';
import { i18n, initializeI18n } from '@/shared/i18n/i18n';
import { err, ok } from '@/shared/result';

const navigate = vi.hoisted(() => vi.fn());

vi.mock('@/app/app-services', () => ({ useAppServices: vi.fn() }));
vi.mock('@/features/session/ui/session-provider', () => ({ useOptionalSession: vi.fn() }));
vi.mock('@tanstack/react-router', async (importOriginal) => ({
  ...(await importOriginal<typeof import('@tanstack/react-router')>()),
  useNavigate: () => navigate,
}));

const send = vi.fn();
const beginAuthentication = vi.fn();
let account: { exportData: ReturnType<typeof vi.fn>; requestDeletion: ReturnType<typeof vi.fn> };

beforeAll(async () => {
  await initializeI18n(window.localStorage, ['en']);
});

beforeEach(() => {
  account = {
    exportData: vi.fn(),
    requestDeletion: vi.fn(() => Promise.resolve(ok(undefined))),
  };
  vi.mocked(useAppServices).mockReturnValue({
    account: account as unknown as AccountGateway,
    controlPlane: { beginAuthentication },
  } as unknown as ReturnType<typeof useAppServices>);
  window.sessionStorage.clear();
});

afterEach(() => {
  cleanup();
  vi.clearAllMocks();
});

describe('设置里的账户', () => {
  it('很久没登录时先请人重新登录，回来还是这一页', () => {
    signedIn(false);
    render(<AccountSettings />, { wrapper: Translations });

    fireEvent.click(screen.getByRole('button', { name: 'Delete account…' }));

    expect(screen.getByText(/sign in again before deleting/u)).toBeVisible();
    expect(screen.queryByRole('textbox')).not.toBeInTheDocument();
    fireEvent.click(screen.getByRole('button', { name: 'Sign in again' }));
    expect(beginAuthentication).toHaveBeenCalledWith('/settings/account');
    expect(account.requestDeletion).not.toHaveBeenCalled();
  });

  it('勾选并输入 DELETE 才能删；删完退出登录、回首页说一声', async () => {
    signedIn(true);
    render(<AccountSettings />, { wrapper: Translations });
    fireEvent.click(screen.getByRole('button', { name: 'Delete account…' }));
    const confirm = screen.getByRole('button', { name: 'Delete my account' });

    fireEvent.change(screen.getByRole('textbox', { name: 'Type DELETE to confirm' }), {
      target: { value: 'delete' },
    });
    expect(confirm).toBeDisabled();
    fireEvent.click(screen.getByRole('checkbox'));
    expect(confirm).toBeEnabled();
    fireEvent.click(confirm);

    await waitFor(() => {
      expect(send).toHaveBeenCalledWith({ type: 'LOGOUT' });
    });
    expect(account.requestDeletion).toHaveBeenCalledWith(
      expect.stringMatching(
        /^[0-9a-f]{8}-[0-9a-f]{4}-7[0-9a-f]{3}-[89ab][0-9a-f]{3}-[0-9a-f]{12}$/u,
      ),
    );
    expect(navigate).toHaveBeenCalledWith({ to: '/' });

    render(<AccountDeletedNotice />, { wrapper: Translations });
    expect(screen.getByText('Your account is being deleted')).toBeVisible();
  });

  it('服务器要求重新登录时换成重新登录，别的失败照实说', async () => {
    signedIn(true);
    account.requestDeletion
      .mockResolvedValueOnce(err({ code: 'account.dependency_unavailable', retryable: true }))
      .mockResolvedValueOnce(
        err({ code: 'authentication.reauthentication_required', retryable: false }),
      );
    render(<AccountSettings />, { wrapper: Translations });
    fireEvent.click(screen.getByRole('button', { name: 'Delete account…' }));
    fireEvent.click(screen.getByRole('checkbox'));
    fireEvent.change(screen.getByRole('textbox', { name: 'Type DELETE to confirm' }), {
      target: { value: 'DELETE' },
    });

    fireEvent.click(screen.getByRole('button', { name: 'Delete my account' }));
    expect(await screen.findByRole('alert')).toHaveTextContent('account.dependency_unavailable');
    fireEvent.click(screen.getByRole('button', { name: 'Delete my account' }));

    expect(await screen.findByRole('button', { name: 'Sign in again' })).toBeVisible();
    expect(send).not.toHaveBeenCalled();
    expect(account.requestDeletion.mock.calls[0]?.[0]).toBe(
      account.requestDeletion.mock.calls[1]?.[0],
    );
  });

  it('下载我的数据存成一个 JSON 文件', async () => {
    signedIn(true);
    account.exportData.mockResolvedValue(
      ok({ fileName: 'agent-room-account-2026-10-08.json', json: '{}\n' }),
    );
    // jsdom 没有 createObjectURL，这个文件里换成假的。
    Object.defineProperty(URL, 'createObjectURL', {
      configurable: true,
      value: vi.fn(() => 'blob:export'),
    });
    Object.defineProperty(URL, 'revokeObjectURL', { configurable: true, value: vi.fn() });
    const click = vi
      .spyOn(HTMLAnchorElement.prototype, 'click')
      .mockImplementation(() => undefined);
    render(<AccountSettings />, { wrapper: Translations });

    fireEvent.click(screen.getByRole('button', { name: 'Download' }));

    await waitFor(() => {
      expect(click).toHaveBeenCalledTimes(1);
    });
    const anchor = click.mock.contexts[0] as HTMLAnchorElement;
    expect(anchor.download).toBe('agent-room-account-2026-10-08.json');
    expect(anchor.href).toBe('blob:export');
    click.mockRestore();
  });
});

function signedIn(recentlyAuthenticated: boolean) {
  vi.mocked(useOptionalSession).mockReturnValue({
    send,
    snapshot: {
      context: {
        principal: {
          authenticatedAtUnixMs: 1_800_000_000_000,
          displayName: 'Operator',
          expiresAtUnixMs: 1_900_000_000_000,
          locale: 'en',
          matrixUserId: '@operator:matrix.test',
          principalId: '018c251e-7b5a-7c7f-8a28-2de53f56a9a3',
          recentlyAuthenticated,
        },
      },
    },
  } as unknown as ReturnType<typeof useOptionalSession>);
}

function Translations({ children }: { readonly children: ReactNode }) {
  return <I18nextProvider i18n={i18n}>{children}</I18nextProvider>;
}
