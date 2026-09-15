import { err, ok, type Result } from '@/shared/result';
import {
  changeWorkspace,
  emptyWorkspace,
  mergeWorkspaces,
  workspaceDocumentsEqual,
  workspaceIndex,
  type WorkspaceChange,
  type WorkspaceDocument,
  type WorkspaceFailure,
  type WorkspaceIndex,
} from '../domain/workspace-document';
import type { WorkspaceCache, WorkspaceGateway, WorkspaceScope } from '../domain/workspace-gateway';

export type PersonalWorkspaceSnapshot = {
  readonly accountId: string | null;
  readonly document: WorkspaceDocument;
  readonly index: WorkspaceIndex;
  readonly status: 'unavailable' | 'loading' | 'pending' | 'synced' | 'failed';
  readonly failure: WorkspaceFailure | null;
};

export type FavoriteUndo = {
  readonly accountId: string;
  readonly agentId: string;
  readonly previous: boolean;
  readonly clock: number;
  readonly writerId: string;
};

export class PersonalWorkspaceStore {
  readonly #listeners = new Set<() => void>();
  #document = emptyWorkspace;
  #scope: WorkspaceScope | null = null;
  #cacheReady = false;
  #detach: (() => void) | null = null;
  #requested = false;
  #syncing = false;
  #snapshot: PersonalWorkspaceSnapshot = Object.freeze({
    accountId: null,
    document: emptyWorkspace,
    index: workspaceIndex(emptyWorkspace),
    status: 'unavailable',
    failure: null,
  });

  constructor(
    private readonly gateway: WorkspaceGateway,
    private readonly cache: WorkspaceCache,
  ) {}

  readonly getSnapshot = (): PersonalWorkspaceSnapshot => this.#snapshot;
  readonly changeForAccount = (
    accountId: string,
    change: WorkspaceChange,
  ): Result<void, WorkspaceFailure> => {
    if (this.#prepare()?.accountId !== accountId)
      return err({ code: 'workspace.session_changed', retryable: true });
    return this.change(change);
  };
  readonly toggleFavorite = (agentId: string): Result<FavoriteUndo, WorkspaceFailure> => {
    const scope = this.#prepare();
    if (scope === null) return err({ code: 'workspace.sign_in_required', retryable: true });
    const previous = this.#snapshot.index.favorites.has(agentId);
    const changed = this.change({ kind: 'favorite', id: agentId, value: !previous });
    if (!changed.ok) return changed;
    const entry = this.#document.entries.find(
      (item) => item.kind === 'favorite' && item.id === agentId,
    );
    if (!entry) return err({ code: 'workspace.invalid_change', retryable: false });
    return ok({
      accountId: scope.accountId,
      agentId,
      previous,
      clock: entry.clock,
      writerId: entry.writerId,
    });
  };

  readonly undoFavorite = (receipt: FavoriteUndo): Result<void, WorkspaceFailure> => {
    const scope = this.#prepare();
    const current = this.#document.entries.find(
      (entry) => entry.kind === 'favorite' && entry.id === receipt.agentId,
    );
    if (
      scope?.accountId !== receipt.accountId ||
      current?.clock !== receipt.clock ||
      current.writerId !== receipt.writerId
    )
      return err({ code: 'workspace.undo_changed', retryable: false });
    return this.change({ kind: 'favorite', id: receipt.agentId, value: receipt.previous });
  };
  readonly subscribe = (listener: () => void): (() => void) => {
    this.#listeners.add(listener);
    if (this.#listeners.size === 1) {
      this.#detach = this.gateway.subscribe(this.retry);
      this.retry();
    }
    return () => {
      this.#listeners.delete(listener);
      if (this.#listeners.size === 0) {
        this.#detach?.();
        this.#detach = null;
      }
    };
  };

  readonly change = (change: WorkspaceChange): Result<void, WorkspaceFailure> =>
    this.changeMany([change]);

  readonly changeMany = (changes: readonly WorkspaceChange[]): Result<void, WorkspaceFailure> => {
    const scope = this.#prepare();
    if (scope === null) return err({ code: 'workspace.sign_in_required', retryable: true });
    if (!this.#cacheReady)
      return err(
        this.#snapshot.failure ?? { code: 'workspace.cache_read_failed', retryable: true },
      );
    let candidate = this.#document;
    for (const change of changes) {
      const next = changeWorkspace(candidate, change, scope.writerId);
      if (!next.ok) {
        this.#publish('failed', next.error);
        return next;
      }
      candidate = next.value;
    }
    if (workspaceDocumentsEqual(candidate, this.#document)) return ok(undefined);
    const saved = this.cache.write(scope.accountId, candidate);
    if (!saved.ok) {
      this.#publish('failed', saved.error);
      return saved;
    }
    this.#document = candidate;
    this.#publish('pending');
    this.retry();
    return ok(undefined);
  };

  readonly retry = (): void => {
    this.#prepare();
    this.#requested = true;
    if (!this.#syncing && this.#listeners.size > 0) {
      this.#syncing = true;
      void this.#drain();
    }
  };

  async #drain(): Promise<void> {
    let passes = 0;
    try {
      while (this.#requested && this.#listeners.size > 0 && passes < 8) {
        this.#requested = false;
        passes += 1;
        const scope = this.gateway.scope();
        try {
          await this.#reconcile();
        } catch {
          if (scope !== null && !this.#current(scope)) this.#requested = true;
          else this.#publish('failed', { code: 'workspace.sync_failed', retryable: true });
        }
      }
      if (this.#requested && passes === 8) {
        this.#requested = false;
        this.#publish('failed', { code: 'workspace.sync_conflict', retryable: true });
      }
    } finally {
      this.#syncing = false;
    }
  }

  async #reconcile(): Promise<void> {
    const scope = this.#prepare();
    if (scope === null || !this.#cacheReady) return;
    const remote = await this.gateway.read(scope);
    if (!this.#current(scope)) {
      this.#requested = true;
      return;
    }
    if (!remote.ok) {
      this.#publish('failed', remote.error);
      return;
    }
    const merged = mergeWorkspaces(this.#document, remote.value);
    if (!merged.ok) {
      this.#publish('failed', merged.error);
      return;
    }
    if (!this.#save(scope, merged.value)) return;
    if (workspaceDocumentsEqual(this.#document, remote.value)) {
      this.#publish('synced');
      return;
    }
    const candidate = this.#document;
    this.#publish('pending');
    const written = await this.gateway.write(scope, candidate);
    if (!this.#current(scope)) {
      this.#requested = true;
      return;
    }
    if (!written.ok) {
      this.#publish('failed', written.error);
      return;
    }
    const confirmed = await this.gateway.read(scope);
    if (!this.#current(scope)) {
      this.#requested = true;
      return;
    }
    if (!confirmed.ok) {
      this.#publish('failed', confirmed.error);
      return;
    }
    // Include edits made while the write was in flight, not just the submitted snapshot.
    const final = mergeWorkspaces(this.#document, confirmed.value);
    if (!final.ok) {
      this.#publish('failed', final.error);
      return;
    }
    if (!this.#save(scope, final.value)) return;
    if (workspaceDocumentsEqual(this.#document, confirmed.value)) this.#publish('synced');
    else {
      this.#requested = true;
      this.#publish('pending');
    }
  }

  #save(scope: WorkspaceScope, document: WorkspaceDocument): boolean {
    const saved = this.cache.write(scope.accountId, document);
    if (!saved.ok) {
      this.#publish('failed', saved.error);
      return false;
    }
    this.#document = document;
    return true;
  }

  #prepare(): WorkspaceScope | null {
    const scope = this.gateway.scope();
    if (scope?.accountId !== this.#scope?.accountId || scope?.writerId !== this.#scope?.writerId) {
      this.#scope = scope;
      this.#document = emptyWorkspace;
      this.#cacheReady = false;
      this.#publish(scope === null ? 'unavailable' : 'loading');
    }
    if (scope !== null && !this.#cacheReady) {
      const cached = this.cache.read(scope.accountId);
      if (cached.ok) {
        this.#document = cached.value;
        this.#cacheReady = true;
        this.#publish('loading');
      } else this.#publish('failed', cached.error);
    }
    return scope;
  }

  #current(scope: WorkspaceScope): boolean {
    const current = this.gateway.scope();
    return current?.accountId === scope.accountId && current.writerId === scope.writerId;
  }

  #publish(
    status: PersonalWorkspaceSnapshot['status'],
    failure: WorkspaceFailure | null = null,
  ): void {
    this.#snapshot = Object.freeze({
      accountId: this.#scope?.accountId ?? null,
      document: this.#document,
      index: workspaceIndex(this.#document),
      status,
      failure,
    });
    for (const listener of this.#listeners) listener();
  }
}
