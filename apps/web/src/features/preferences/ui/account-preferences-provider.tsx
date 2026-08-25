import {
  createContext,
  useContext,
  useEffect,
  useMemo,
  useSyncExternalStore,
  type PropsWithChildren,
} from 'react';

import type {
  AccountPreferencesSnapshot,
  AccountPreferencesStore,
} from '@/features/preferences/application/account-preferences-store';
import type {
  LanguagePreference,
  LobbyViewPreference,
} from '@/features/preferences/domain/account-preferences';
import type { AccountPreferencesSyncFailure } from '@/features/preferences/domain/account-preferences-gateway';
import { setLanguagePreference } from '@/shared/i18n/i18n';
import type { Result } from '@/shared/result';

export type AccountPreferencesContextValue = {
  readonly retry: () => void;
  readonly setLanguage: (value: LanguagePreference) => Result<void, AccountPreferencesSyncFailure>;
  readonly setLobbyView: (
    value: LobbyViewPreference,
  ) => Result<void, AccountPreferencesSyncFailure>;
  readonly snapshot: AccountPreferencesSnapshot;
};

const AccountPreferencesContext = createContext<AccountPreferencesContextValue | null>(null);

export type AccountPreferencesProviderProps = PropsWithChildren<{
  readonly store: AccountPreferencesStore;
}>;

export function AccountPreferencesProvider({ children, store }: AccountPreferencesProviderProps) {
  const snapshot = useSyncExternalStore(store.subscribe, store.getSnapshot, store.getSnapshot);

  useEffect(() => {
    void setLanguagePreference(snapshot.values.language);
  }, [snapshot.values.language]);

  const value = useMemo<AccountPreferencesContextValue>(
    () => ({
      retry: store.retry,
      setLanguage: (language) => store.update('language', language),
      setLobbyView: (lobbyView) => store.update('lobbyView', lobbyView),
      snapshot,
    }),
    [snapshot, store],
  );

  return (
    <AccountPreferencesContext.Provider value={value}>
      {children}
    </AccountPreferencesContext.Provider>
  );
}

export function useAccountPreferences(): AccountPreferencesContextValue {
  const value = useContext(AccountPreferencesContext);
  if (value === null) {
    throw new Error('AccountPreferencesProvider is missing.');
  }
  return value;
}

export function useOptionalAccountPreferences(): AccountPreferencesContextValue | null {
  return useContext(AccountPreferencesContext);
}
