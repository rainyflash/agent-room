import { ClientEvent, type MatrixClient, type MatrixEvent } from 'matrix-js-sdk';

import {
  parseAccountPreferencesDocument,
  type AccountPreferencesDocument,
} from '@/features/preferences/domain/account-preferences';
import type {
  AccountPreferencesGateway,
  AccountPreferencesScope,
  AccountPreferencesSyncFailure,
} from '@/features/preferences/domain/account-preferences-gateway';
import type { MatrixClientSource } from '@/shared/matrix/matrix-client-registry';
import { err, ok, type Result } from '@/shared/result';

export const ACCOUNT_PREFERENCES_EVENT_TYPE = 'org.agentroom.preferences.v1';

type CustomAccountDataClient = {
  getAccountDataFromServer(eventType: typeof ACCOUNT_PREFERENCES_EVENT_TYPE): Promise<unknown>;
  setAccountData(
    eventType: typeof ACCOUNT_PREFERENCES_EVENT_TYPE,
    content: AccountPreferencesDocument,
  ): Promise<unknown>;
};

export class MatrixAccountPreferencesGateway implements AccountPreferencesGateway {
  readonly #listeners = new Set<() => void>();
  readonly #source: MatrixClientSource;
  #boundClient: MatrixClient | null = null;
  #detachSource: (() => void) | null = null;

  constructor(source: MatrixClientSource) {
    this.#source = source;
  }

  async read(): Promise<Result<AccountPreferencesDocument | null, AccountPreferencesSyncFailure>> {
    const client = this.#source.current();
    if (client === null || this.scope() === null) {
      return err({ code: 'preferences.source_unavailable', retryable: true });
    }
    try {
      const content = await accountDataClient(client).getAccountDataFromServer(
        ACCOUNT_PREFERENCES_EVENT_TYPE,
      );
      return content === null ? ok(null) : parseAccountPreferencesDocument(content);
    } catch {
      return err({ code: 'preferences.read_failed', retryable: true });
    }
  }

  scope(): AccountPreferencesScope | null {
    const client = this.#source.current();
    const accountId = client?.getUserId() ?? null;
    const writerId = client?.getDeviceId() ?? null;
    return client === null || accountId === null || writerId === null
      ? null
      : Object.freeze({ accountId, writerId });
  }

  subscribe(listener: () => void): () => void {
    this.#listeners.add(listener);
    if (this.#listeners.size === 1) {
      this.#detachSource = this.#source.subscribe(this.#handleSourceActivity);
      this.#bindClient(this.#source.current());
    }
    return () => {
      this.#listeners.delete(listener);
      if (this.#listeners.size === 0) {
        this.#detachSource?.();
        this.#detachSource = null;
        this.#bindClient(null);
      }
    };
  }

  async write(
    document: AccountPreferencesDocument,
  ): Promise<Result<void, AccountPreferencesSyncFailure>> {
    const client = this.#source.current();
    if (client === null || this.scope() === null) {
      return err({ code: 'preferences.source_unavailable', retryable: true });
    }
    try {
      await accountDataClient(client).setAccountData(ACCOUNT_PREFERENCES_EVENT_TYPE, document);
      return ok(undefined);
    } catch {
      return err({ code: 'preferences.write_failed', retryable: true });
    }
  }

  readonly #handleAccountData = (event: MatrixEvent): void => {
    if (event.getType() === ACCOUNT_PREFERENCES_EVENT_TYPE) {
      this.#notify();
    }
  };

  readonly #handleSourceActivity = (): void => {
    this.#bindClient(this.#source.current());
    this.#notify();
  };

  #bindClient(client: MatrixClient | null): void {
    if (client === this.#boundClient) {
      return;
    }
    this.#boundClient?.removeListener(ClientEvent.AccountData, this.#handleAccountData);
    this.#boundClient = client;
    this.#boundClient?.on(ClientEvent.AccountData, this.#handleAccountData);
  }

  #notify(): void {
    for (const listener of this.#listeners) {
      listener();
    }
  }
}

function accountDataClient(client: MatrixClient): CustomAccountDataClient {
  return client as unknown as CustomAccountDataClient;
}
