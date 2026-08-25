import {
  accountPreferencesDocumentsEqual,
  createAccountPreferencesDocument,
  mergeAccountPreferencesDocuments,
  updateAccountPreference,
  valuesFromAccountPreferences,
  type AccountPreferenceValues,
  type AccountPreferencesDocument,
} from '@/features/preferences/domain/account-preferences';
import type {
  AccountPreferencesGateway,
  AccountPreferencesScope,
  AccountPreferencesSyncFailure,
} from '@/features/preferences/domain/account-preferences-gateway';
import { ok, type Result } from '@/shared/result';

export type AccountPreferencesSyncStatus = 'failed' | 'loading' | 'local' | 'pending' | 'synced';

export type AccountPreferencesSnapshot = {
  readonly failure: AccountPreferencesSyncFailure | null;
  readonly status: AccountPreferencesSyncStatus;
  readonly values: AccountPreferenceValues;
};

export class AccountPreferencesStore {
  readonly #gateway: AccountPreferencesGateway;
  readonly #listeners = new Set<() => void>();
  #accountId: string | null = null;
  #bootstrapValues: AccountPreferenceValues;
  #detachGateway: (() => void) | null = null;
  #document: AccountPreferencesDocument | null = null;
  #pending = false;
  #reconcileRequested = false;
  #reconciling = false;
  #snapshot: AccountPreferencesSnapshot;

  constructor(gateway: AccountPreferencesGateway, bootstrapValues: AccountPreferenceValues) {
    this.#gateway = gateway;
    this.#bootstrapValues = Object.freeze({ ...bootstrapValues });
    this.#snapshot = freezeSnapshot({
      failure: null,
      status: 'loading',
      values: this.#bootstrapValues,
    });
  }

  readonly getSnapshot = (): AccountPreferencesSnapshot => {
    return this.#snapshot;
  };

  readonly retry = (): void => {
    this.#requestReconciliation();
  };

  readonly subscribe = (listener: () => void): (() => void) => {
    this.#listeners.add(listener);
    if (this.#listeners.size === 1) {
      this.#detachGateway = this.#gateway.subscribe(this.#requestReconciliation);
      this.#requestReconciliation();
    }
    return () => {
      this.#listeners.delete(listener);
      if (this.#listeners.size === 0) {
        this.#detachGateway?.();
        this.#detachGateway = null;
      }
    };
  };

  update<TKey extends keyof AccountPreferenceValues>(
    key: TKey,
    value: AccountPreferenceValues[TKey],
  ): Result<void, AccountPreferencesSyncFailure> {
    this.#bootstrapValues = Object.freeze({ ...this.#bootstrapValues, [key]: value });
    const scope = this.#prepareScope();
    if (scope === null) {
      this.#replaceSnapshot({ failure: null, status: 'local', values: this.#bootstrapValues });
      return ok(undefined);
    }
    const base =
      this.#document === null
        ? createAccountPreferencesDocument(this.#bootstrapValues, scope.writerId)
        : ok(this.#document);
    if (!base.ok) {
      this.#replaceSnapshot({
        failure: base.error,
        status: 'failed',
        values: this.#bootstrapValues,
      });
      return base;
    }
    const updated = updateAccountPreference(base.value, key, value, scope.writerId);
    if (!updated.ok) {
      this.#replaceSnapshot({
        failure: updated.error,
        status: 'failed',
        values: this.#bootstrapValues,
      });
      return updated;
    }
    this.#document = updated.value;
    this.#pending = true;
    this.#publishDocument('pending', null);
    this.#requestReconciliation();
    return ok(undefined);
  }

  readonly #requestReconciliation = (): void => {
    this.#reconcileRequested = true;
    if (this.#listeners.size === 0 || this.#reconciling) {
      return;
    }
    this.#reconciling = true;
    void this.#drainReconciliation();
  };

  async #drainReconciliation(): Promise<void> {
    try {
      while (this.#reconcileRequested && this.#listeners.size > 0) {
        this.#reconcileRequested = false;
        await this.#reconcileOnce();
      }
    } catch {
      this.#publishFailure({ code: 'preferences.unexpected_failure', retryable: true });
    } finally {
      this.#reconciling = false;
      if (this.#reconcileRequested && this.#listeners.size > 0) {
        this.#requestReconciliation();
      }
    }
  }

  async #reconcileOnce(): Promise<void> {
    const scope = this.#prepareScope();
    if (scope === null) {
      this.#replaceSnapshot({ failure: null, status: 'local', values: this.#bootstrapValues });
      return;
    }
    const remote = await this.#gateway.read();
    if (!this.#scopeIsCurrent(scope)) {
      this.#reconcileRequested = true;
      return;
    }
    if (!remote.ok) {
      this.#publishFailure(remote.error);
      return;
    }
    const candidate = this.#mergeRemote(remote.value, scope);
    if (!candidate.ok) {
      this.#publishFailure(candidate.error);
      return;
    }
    this.#document = candidate.value;
    if (remote.value !== null && accountPreferencesDocumentsEqual(candidate.value, remote.value)) {
      this.#pending = false;
      this.#publishDocument('synced', null);
      return;
    }
    this.#pending = true;
    this.#publishDocument('pending', null);
    const written = await this.#gateway.write(candidate.value);
    if (!this.#scopeIsCurrent(scope)) {
      this.#reconcileRequested = true;
      return;
    }
    if (!written.ok) {
      this.#publishFailure(written.error);
      return;
    }
    await this.#confirmWrite(scope, candidate.value);
  }

  async #confirmWrite(
    scope: AccountPreferencesScope,
    candidate: AccountPreferencesDocument,
  ): Promise<void> {
    const confirmed = await this.#gateway.read();
    if (!this.#scopeIsCurrent(scope)) {
      this.#reconcileRequested = true;
      return;
    }
    if (!confirmed.ok) {
      this.#publishFailure(confirmed.error);
      return;
    }
    if (confirmed.value === null) {
      this.#publishFailure({ code: 'preferences.confirmation_missing', retryable: true });
      return;
    }
    const merged = mergeAccountPreferencesDocuments(candidate, confirmed.value);
    this.#document = merged;
    if (accountPreferencesDocumentsEqual(merged, confirmed.value)) {
      this.#pending = false;
      this.#publishDocument('synced', null);
      return;
    }
    this.#pending = true;
    this.#publishDocument('pending', null);
    this.#reconcileRequested = true;
  }

  #mergeRemote(
    remote: AccountPreferencesDocument | null,
    scope: AccountPreferencesScope,
  ): Result<AccountPreferencesDocument, AccountPreferencesSyncFailure> {
    if (remote === null) {
      return this.#document === null
        ? createAccountPreferencesDocument(this.#bootstrapValues, scope.writerId)
        : ok(this.#document);
    }
    return ok(
      this.#document === null ? remote : mergeAccountPreferencesDocuments(remote, this.#document),
    );
  }

  #prepareScope(): AccountPreferencesScope | null {
    const scope = this.#gateway.scope();
    if (scope === null) {
      this.#accountId = null;
      this.#document = null;
      this.#pending = false;
      return null;
    }
    if (scope.accountId !== this.#accountId) {
      this.#accountId = scope.accountId;
      this.#document = null;
      this.#pending = false;
      this.#replaceSnapshot({
        failure: null,
        status: 'loading',
        values: this.#bootstrapValues,
      });
    }
    return scope;
  }

  #scopeIsCurrent(expected: AccountPreferencesScope): boolean {
    const current = this.#gateway.scope();
    return (
      current !== null &&
      current.accountId === expected.accountId &&
      current.writerId === expected.writerId
    );
  }

  #publishDocument(
    status: AccountPreferencesSyncStatus,
    failure: AccountPreferencesSyncFailure | null,
  ): void {
    const document = this.#document;
    if (document === null) {
      this.#publishFailure(failure ?? { code: 'preferences.unexpected_failure', retryable: true });
      return;
    }
    this.#bootstrapValues = valuesFromAccountPreferences(document);
    this.#replaceSnapshot({ failure, status, values: this.#bootstrapValues });
  }

  #publishFailure(failure: AccountPreferencesSyncFailure): void {
    const status: AccountPreferencesSyncStatus = this.#pending ? 'pending' : 'failed';
    this.#replaceSnapshot({ failure, status, values: this.#snapshot.values });
  }

  #replaceSnapshot(snapshot: AccountPreferencesSnapshot): void {
    const frozen = freezeSnapshot(snapshot);
    if (snapshotsEqual(this.#snapshot, frozen)) {
      return;
    }
    this.#snapshot = frozen;
    for (const listener of this.#listeners) {
      listener();
    }
  }
}

function freezeSnapshot(snapshot: AccountPreferencesSnapshot): AccountPreferencesSnapshot {
  return Object.freeze({
    failure: snapshot.failure === null ? null : Object.freeze({ ...snapshot.failure }),
    status: snapshot.status,
    values: Object.freeze({ ...snapshot.values }),
  });
}

function snapshotsEqual(
  left: AccountPreferencesSnapshot,
  right: AccountPreferencesSnapshot,
): boolean {
  return (
    left.status === right.status &&
    left.failure?.code === right.failure?.code &&
    left.failure?.retryable === right.failure?.retryable &&
    left.values.language === right.values.language &&
    left.values.lobbyView === right.values.lobbyView
  );
}
