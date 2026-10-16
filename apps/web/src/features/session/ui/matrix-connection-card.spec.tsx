// @vitest-environment jsdom
import '@testing-library/jest-dom/vitest';
import { cleanup, fireEvent, render, screen } from '@testing-library/react';
import { I18nextProvider } from 'react-i18next';
import { afterEach, beforeAll, describe, expect, it, vi } from 'vitest';

import type { SessionFailure, WebSession } from '../domain/session';
import type { SessionContext } from '../domain/session-machine';
import { MatrixConnectionCard } from './matrix-connection-card';
import { MatrixConnectionToast } from './matrix-connection-toast';
import { initializeI18n, i18n } from '@/shared/i18n/i18n';

const principal: WebSession = {
  authenticatedAtUnixMs: 1_700_000_000_000,
  displayName: 'Local Developer',
  expiresAtUnixMs: 1_700_028_800_000,
  locale: 'en',
  matrixUserId: '@user-0123456789abcdef:matrix.agent-room.test',
  principalId: '018c251e-7b5a-7c7f-8a28-2de53f56a9a3',
  recentlyAuthenticated: true,
};

const timedOut: SessionFailure = {
  boundary: 'matrix',
  code: 'matrix_session.unavailable',
  offline: false,
  retryable: true,
};

const session = vi.hoisted(() => ({
  current: null as null | {
    readonly send: (event: { readonly type: string }) => void;
    readonly snapshot: { readonly value: string; readonly context: unknown };
  },
}));

vi.mock('./session-provider', () => ({
  useOptionalSession: () => session.current,
  useSession: () => {
    if (session.current === null) throw new Error('SessionProvider is missing.');
    return session.current;
  },
}));
vi.mock('@tanstack/react-router', () => ({
  Link: ({ children, to }: { readonly children: React.ReactNode; readonly to: string }) => (
    <a href={to}>{children}</a>
  ),
}));

function givenSession(value: string, overrides: Partial<SessionContext> = {}) {
  const send = vi.fn();
  const context: SessionContext = {
    authenticationMode: 'automatic',
    authenticationTarget: 'matrix',
    connection: null,
    controlStatus: 'ready',
    failure: null,
    principal,
    resumePath: null,
    ...overrides,
  };
  session.current = { send, snapshot: { value, context } };
  return send;
}

function renderWithI18n(node: React.ReactNode) {
  return render(<I18nextProvider i18n={i18n}>{node}</I18nextProvider>);
}

beforeAll(async () => {
  await initializeI18n(window.localStorage, ['en']);
});
afterEach(() => {
  cleanup();
  session.current = null;
});

describe('房间页：消息没连上时的卡片', () => {
  it('聊天登录没拿到：说消息没连上，“重新连接”交给会话', () => {
    const send = givenSession('degraded', { failure: timedOut });
    renderWithI18n(<MatrixConnectionCard fallback={<p>fallback</p>} />);

    expect(screen.getByText('Messages are not connected')).toBeVisible();
    expect(screen.getByText('matrix_session.unavailable')).toBeInTheDocument();
    fireEvent.click(screen.getByRole('button', { name: 'Reconnect' }));
    expect(send).toHaveBeenCalledWith({ type: 'RETRY' });
    expect(screen.queryByText('fallback')).toBeNull();
  });

  it.each(['restoring', 'authenticating'])(
    '%s 的时候只说正在接通，不给按钮：桌面端的聊天登录由控制面发，不用去浏览器',
    (state) => {
      givenSession(state);
      renderWithI18n(<MatrixConnectionCard fallback={<p>fallback</p>} />);

      expect(screen.getByText('Connecting messages')).toBeVisible();
      expect(screen.queryByRole('button')).toBeNull();
    },
  );

  it('没有会话（夹具）时照旧显示原来的卡片', () => {
    renderWithI18n(<MatrixConnectionCard fallback={<p>fallback</p>} />);

    expect(screen.getByText('fallback')).toBeVisible();
  });
});

describe('提示栈：消息没连上', () => {
  it('要人动手时常驻一条，按钮交给会话', () => {
    const send = givenSession('degraded', { failure: timedOut });
    renderWithI18n(<MatrixConnectionToast hidden={false} />);

    expect(screen.getByText('Messages are not connected')).toBeVisible();
    fireEvent.click(screen.getByRole('button', { name: 'Reconnect' }));
    expect(send).toHaveBeenCalledWith({ type: 'RETRY' });
  });

  it('页面自己会说、或者正在连时不提示', () => {
    givenSession('degraded', { failure: timedOut });
    const { container, rerender } = renderWithI18n(<MatrixConnectionToast hidden />);
    expect(container).toBeEmptyDOMElement();

    givenSession('syncing');
    rerender(
      <I18nextProvider i18n={i18n}>
        <MatrixConnectionToast hidden={false} />
      </I18nextProvider>,
    );
    expect(container).toBeEmptyDOMElement();
  });
});
