import {
  createContext,
  useContext,
  useEffect,
  useMemo,
  useState,
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
import {
  readDeviceLanguageOverride,
  setDeviceLanguageOverride as applyDeviceLanguageOverride,
  setLanguagePreference,
} from '@/shared/i18n/i18n';
import type { DeviceLanguageOverride } from '@/shared/i18n/language';
import type { Result } from '@/shared/result';

export type AccountPreferencesContextValue = {
  readonly deviceLanguageOverride: DeviceLanguageOverride;
  readonly retry: () => void;
  readonly setDeviceLanguageOverride: (value: DeviceLanguageOverride) => void;
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
  const [deviceLanguageOverride, setDeviceLanguageOverrideState] = useState<DeviceLanguageOverride>(
    () => readDeviceLanguageOverride(window.localStorage),
  );

  useEffect(() => {
    void setLanguagePreference(snapshot.values.language);
  }, [deviceLanguageOverride, snapshot.values.language]);

  const value = useMemo<AccountPreferencesContextValue>(
    () => ({
      deviceLanguageOverride,
      retry: store.retry,
      setDeviceLanguageOverride: (override) => {
        setDeviceLanguageOverrideState(override);
        void applyDeviceLanguageOverride(override, snapshot.values.language);
      },
      setLanguage: (language) => store.update('language', language),
      setLobbyView: (lobbyView) => store.update('lobbyView', lobbyView),
      snapshot,
    }),
    [deviceLanguageOverride, snapshot, store],
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
