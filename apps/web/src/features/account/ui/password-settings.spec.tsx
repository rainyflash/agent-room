// @vitest-environment jsdom

import '@testing-library/jest-dom/vitest';

import { cleanup, fireEvent, render, screen, waitFor } from '@testing-library/react';
import type { ReactNode } from 'react';
import { I18nextProvider } from 'react-i18next';
import { afterEach, beforeAll, beforeEach, describe, expect, it, type Mock, vi } from 'vitest';

import type { AccountGateway } from '@/features/account/domain/account';
import { type ChatConnection, PasswordRow } from '@/features/account/ui/password-settings';
import { i18n, initializeI18n } from '@/shared/i18n/i18n';
import { err, ok } from '@/shared/result';

const navigate = vi.hoisted(() => vi.fn());

vi.mock('@tanstack/react-router', async (importOriginal) => ({
  ...(await importOriginal<typeof import('@tanstack/react-router')>()),
  useNavigate: () => navigate,
}));

const PENDING_KEY = 'agent-room.password-changed.pending';
const READY: ChatConnection = { deviceId: 'THISDEVICE', state: 'ready' };
let signOutElsewhere: Mock<AccountGateway['signOutElsewhere']>;
let account: AccountGateway;

beforeAll(async () => {
  await initializeI18n(window.localStorage, ['en']);
});

beforeEach(() => {
  signOutElsewhere = vi.fn<AccountGateway['signOutElsewhere']>(() =>
    Promise.resolve(ok({ accountSessions: 2, chatDevices: 1 })),
  );
  account = {
    exportData: vi.fn<AccountGateway['exportData']>(),
    requestDeletion: vi.fn<AccountGateway['requestDeletion']>(),
    signOutElsewhere,
  };
  window.sessionStorage.clear();
  window.history.replaceState(null, '', '/settings/account');
});

afterEach(() => {
  cleanup();
  vi.clearAllMocks();
});

describe('设置里的修改密码', () => {
  it('点修改密码去身份服务改；没去成时照实说', async () => {
    const begin = vi.fn(() => Promise.resolve({ code: 'authentication.unavailable' }));
    renderRow({ beginChangePassword: begin });

    expect(screen.getByText('Password')).toBeVisible();
    fireEvent.click(screen.getByRole('button', { name: 'Change password' }));

    expect(begin).toHaveBeenCalledTimes(1);
    expect(await screen.findByRole('alert')).toHaveTextContent('(authentication.unavailable)');
    expect(screen.getByRole('button', { name: 'Change password' })).toBeEnabled();
    expect(signOutElsewhere).not.toHaveBeenCalled();
  });

  it('桌面端等人在浏览器里改的时候说一声去哪儿改', () => {
    renderRow({ beginChangePassword: () => new Promise(() => undefined), desktop: true });

    fireEvent.click(screen.getByRole('button', { name: 'Change password' }));

    expect(screen.getByRole('button', { name: 'Opening…' })).toBeDisabled();
    expect(screen.getByRole('status')).toHaveTextContent('browser page that just opened');
  });

  it('改好密码回来：去掉地址上的标记，留着这一台的聊天登录退出别的地方，只退一次', async () => {
    window.history.replaceState(null, '', '/settings/account?passwordChanged=1');
    const view = renderRow();

    expect(
      await screen.findByText(
        'Password changed. You’re signed out in every other browser and on every other computer.',
      ),
    ).toBeVisible();
    expect(screen.getByText(/Authorized computers and agent connections/u)).toBeVisible();
    expect(signOutElsewhere).toHaveBeenCalledTimes(1);
    expect(signOutElsewhere).toHaveBeenCalledWith('THISDEVICE');
    expect(navigate).toHaveBeenCalledWith({
      params: { section: 'account' },
      replace: true,
      search: {},
      to: '/settings/$section',
    });
    expect(window.sessionStorage.getItem(PENDING_KEY)).toBeNull();

    view.rerender(<PasswordRow {...props()} />);
    expect(signOutElsewhere).toHaveBeenCalledTimes(1);
  });

  it('聊天还在连时先等，连上了带着这一台的设备号去退', async () => {
    window.history.replaceState(null, '', '/settings/account?passwordChanged=1');
    const view = renderRow({ chat: { deviceId: null, state: 'restoring' } });

    expect(screen.getByText('Password changed. Signing you out everywhere else…')).toBeVisible();
    expect(signOutElsewhere).not.toHaveBeenCalled();
    expect(window.sessionStorage.getItem(PENDING_KEY)).toBe('1');

    view.rerender(<PasswordRow {...props({ chat: READY })} />);

    await waitFor(() => {
      expect(signOutElsewhere).toHaveBeenCalledWith('THISDEVICE');
    });
  });

  it('聊天连不上就不留这一台，连它一起退', async () => {
    window.history.replaceState(null, '', '/settings/account?passwordChanged=1');
    renderRow({ chat: { deviceId: null, state: 'degraded' } });

    await waitFor(() => {
      expect(signOutElsewhere).toHaveBeenCalledWith(null);
    });
  });

  it('页面跳走又回来（比如去聊天登录）还记得要退出别的地方', async () => {
    window.sessionStorage.setItem(PENDING_KEY, '1');
    renderRow();

    await waitFor(() => {
      expect(signOutElsewhere).toHaveBeenCalledTimes(1);
    });
    expect(navigate).not.toHaveBeenCalled();
  });

  it('没退成时说密码已经改好了、给再试一次', async () => {
    window.history.replaceState(null, '', '/settings/account?passwordChanged=1');
    signOutElsewhere.mockImplementationOnce(() =>
      Promise.resolve(err({ code: 'sign_out_elsewhere.chat_unavailable', retryable: true })),
    );
    renderRow();

    expect(
      await screen.findByText('Password changed, but you’re not signed out everywhere else yet.'),
    ).toBeVisible();
    expect(screen.getByText(/sign_out_elsewhere\.chat_unavailable/u)).toBeVisible();
    expect(window.sessionStorage.getItem(PENDING_KEY)).toBe('1');

    fireEvent.click(screen.getByRole('button', { name: 'Try again' }));

    expect(
      await screen.findByText(
        'Password changed. You’re signed out in every other browser and on every other computer.',
      ),
    ).toBeVisible();
    expect(signOutElsewhere).toHaveBeenCalledTimes(2);
    expect(window.sessionStorage.getItem(PENDING_KEY)).toBeNull();
  });

  it('平常打开设置不退出别的地方', () => {
    renderRow();

    expect(screen.queryByRole('status')).toBeNull();
    expect(signOutElsewhere).not.toHaveBeenCalled();
    expect(navigate).not.toHaveBeenCalled();
  });
});

type RowProps = Parameters<typeof PasswordRow>[0];

function props(overrides: Partial<RowProps> = {}): RowProps {
  return {
    account,
    beginChangePassword: () => Promise.resolve(null),
    chat: READY,
    desktop: false,
    ...overrides,
  };
}

function renderRow(overrides: Partial<RowProps> = {}) {
  return render(<PasswordRow {...props(overrides)} />, { wrapper: Translations });
}

function Translations({ children }: { readonly children: ReactNode }) {
  return <I18nextProvider i18n={i18n}>{children}</I18nextProvider>;
}
