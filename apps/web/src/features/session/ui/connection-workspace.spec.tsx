// @vitest-environment jsdom

import '@testing-library/jest-dom/vitest';

import { cleanup, fireEvent, render, screen } from '@testing-library/react';
import { I18nextProvider } from 'react-i18next';
import { afterEach, beforeAll, expect, it, vi } from 'vitest';

import type { SessionContext } from '../domain/session-machine';
import { connectionViewModel } from './connection-model';
import { ConnectionWorkspace } from './connection-workspace';
import { RouterTestProvider } from '@/test/router-test-provider';
import { i18n, initializeI18n } from '@/shared/i18n/i18n';
import { ok } from '@/shared/result';

beforeAll(async () => {
  await initializeI18n(window.localStorage, ['en-US']);
});
afterEach(cleanup);

it('连接正常时可以主动退出，而不必先进入故障状态', () => {
  const context: SessionContext = {
    authenticationMode: 'automatic',
    authenticationTarget: 'control',
    connection: {
      deviceId: 'TESTDEVICE',
      disconnect: () => undefined,
      observe: () => () => undefined,
      userId: '@operator:matrix.test',
      waitUntilPrepared: () => Promise.resolve(ok(undefined)),
    },
    controlStatus: 'ready',
    failure: null,
    principal: {
      authenticatedAtUnixMs: 1_800_000_000_000,
      displayName: 'Operator',
      expiresAtUnixMs: 1_900_000_000_000,
      locale: 'en',
      matrixUserId: '@operator:matrix.test',
      principalId: '018c251e-7b5a-7c7f-8a28-2de53f56a9a3',
      recentlyAuthenticated: true,
    },
    resumePath: null,
  };
  const onAction = vi.fn();
  render(
    <I18nextProvider i18n={i18n}>
      <RouterTestProvider>
        <ConnectionWorkspace
          context={context}
          onAction={onAction}
          view={connectionViewModel('ready', context)}
        >
          {null}
        </ConnectionWorkspace>
      </RouterTestProvider>
    </I18nextProvider>,
  );
  expect(screen.getByRole('button', { name: 'Explore rooms' })).toBeEnabled();
  expect(screen.getByText('TESTDEVICE')).not.toBeVisible();
  expect(screen.getByText('@operator:matrix.test')).not.toBeVisible();
  fireEvent.click(screen.getByText('Connection and identity details'));
  expect(screen.getByText('TESTDEVICE')).toBeVisible();
  expect(screen.getByText('@operator:matrix.test')).toBeVisible();
  // 账户 ID 不必先进某个房间才看得到：邀请你的人需要它。
  const writeText = vi.fn(() => Promise.resolve());
  vi.stubGlobal('navigator', { clipboard: { writeText } });
  try {
    expect(screen.getByText('018c251e-7b5a-7c7f-8a28-2de53f56a9a3')).toBeVisible();
    fireEvent.click(screen.getByRole('button', { name: 'Copy' }));
    expect(writeText).toHaveBeenCalledWith('018c251e-7b5a-7c7f-8a28-2de53f56a9a3');
  } finally {
    vi.unstubAllGlobals();
  }
  fireEvent.click(screen.getByRole('button', { name: 'Sign out' }));
  expect(onAction).toHaveBeenCalledExactlyOnceWith('logout');
});

it('等待浏览器登录时可以重新开始，而不是干等 15 分钟', () => {
  const context: SessionContext = {
    authenticationMode: 'interactive',
    authenticationTarget: 'control',
    connection: null,
    controlStatus: 'unavailable',
    failure: null,
    principal: null,
    resumePath: null,
  };
  const onAction = vi.fn();
  render(
    <I18nextProvider i18n={i18n}>
      <RouterTestProvider>
        <ConnectionWorkspace
          context={context}
          onAction={onAction}
          view={connectionViewModel('authenticating', context)}
        >
          {null}
        </ConnectionWorkspace>
      </RouterTestProvider>
    </I18nextProvider>,
  );
  fireEvent.click(screen.getByRole('button', { name: 'Start sign-in again' }));
  expect(onAction).toHaveBeenCalledExactlyOnceWith('retry');
});
