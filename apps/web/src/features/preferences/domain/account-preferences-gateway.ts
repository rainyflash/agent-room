import type { AccountPreferencesDocument, AccountPreferencesFailure } from './account-preferences';
import type { Result } from '@/shared/result';

export type AccountPreferencesScope = {
  readonly accountId: string;
  readonly writerId: string;
};

export type AccountPreferencesSyncFailure =
  | AccountPreferencesFailure
  | {
      readonly code:
        | 'preferences.confirmation_missing'
        | 'preferences.read_failed'
        | 'preferences.source_unavailable'
        | 'preferences.unexpected_failure'
        | 'preferences.write_failed';
      readonly retryable: boolean;
    };

export type AccountPreferencesGateway = {
  read(): Promise<Result<AccountPreferencesDocument | null, AccountPreferencesSyncFailure>>;
  scope(): AccountPreferencesScope | null;
  subscribe(listener: () => void): () => void;
  write(document: AccountPreferencesDocument): Promise<Result<void, AccountPreferencesSyncFailure>>;
};
