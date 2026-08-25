// @vitest-environment jsdom

import '@testing-library/jest-dom/vitest';

import { render, screen, waitFor } from '@testing-library/react';
import userEvent from '@testing-library/user-event';
import { afterEach, beforeEach, describe, expect, it } from 'vitest';
import { I18nextProvider } from 'react-i18next';

import { AccountPreferencesProvider, useAccountPreferences } from './account-preferences-provider';
import { AccountPreferencesStore } from '@/features/preferences/application/account-preferences-store';
import type { AccountPreferencesGateway } from '@/features/preferences/domain/account-preferences-gateway';
import { LanguageControl } from '@/features/preferences/ui/language-control';
import { i18n, initializeI18n, setLanguagePreference } from '@/shared/i18n/i18n';
import { err } from '@/shared/result';

const localGateway: AccountPreferencesGateway = {
  read: async () => err({ code: 'preferences.source_unavailable', retryable: true }),
  scope: () => null,
  subscribe: () => () => undefined,
  write: async () => err({ code: 'preferences.source_unavailable', retryable: true }),
};

beforeEach(async () => {
  window.localStorage.clear();
  await initializeI18n(window.localStorage, ['en']);
});

afterEach(async () => {
  await setLanguagePreference('en');
  window.localStorage.clear();
});

describe('账户偏好 React 边界', () => {
  it('语言与大厅视图共享同一个仓库，未登录时仍保留可用的本地体验', async () => {
    const store = new AccountPreferencesStore(localGateway, {
      language: 'system',
      lobbyView: 'scene',
    });
    const user = userEvent.setup();

    render(
      <I18nextProvider i18n={i18n}>
        <AccountPreferencesProvider store={store}>
          <LanguageControl />
          <LobbyViewProbe />
        </AccountPreferencesProvider>
      </I18nextProvider>,
    );

    await user.selectOptions(screen.getByRole('combobox'), 'zh-CN');
    await waitFor(() => {
      expect(document.documentElement.lang).toBe('zh-CN');
    });
    expect(window.localStorage.getItem('agent-room.language')).toBe('zh-CN');
    expect(store.getSnapshot().values.language).toBe('zh-CN');

    await user.click(screen.getByRole('button', { name: '切换列表视图' }));
    expect(screen.getByRole('status')).toHaveTextContent('list');
    expect(store.getSnapshot().values.lobbyView).toBe('list');
  });
});

function LobbyViewProbe() {
  const preferences = useAccountPreferences();
  return (
    <>
      <button
        onClick={() => {
          preferences.setLobbyView('list');
        }}
        type="button"
      >
        切换列表视图
      </button>
      <output role="status">{preferences.snapshot.values.lobbyView}</output>
    </>
  );
}
