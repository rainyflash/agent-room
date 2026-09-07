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
  fireEvent.click(screen.getByRole('button', { name: 'Sign out' }));
  expect(onAction).toHaveBeenCalledExactlyOnceWith('logout');
});
