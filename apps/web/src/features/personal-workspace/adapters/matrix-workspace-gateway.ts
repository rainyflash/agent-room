import { ClientEvent, SyncState, type MatrixClient, type MatrixEvent } from 'matrix-js-sdk';
import type { MatrixClientSource } from '@/shared/matrix/matrix-client-registry';
import { err, ok } from '@/shared/result';
import {
  emptyWorkspace,
  parseWorkspace,
  type WorkspaceDocument,
} from '../domain/workspace-document';
import type { WorkspaceGateway, WorkspaceScope } from '../domain/workspace-gateway';

export const PERSONAL_WORKSPACE_EVENT_TYPE = 'io.github.rainyflash.agentroom.personal-workspace.v1';

declare module 'matrix-js-sdk/lib/@types/event' {
  // The SDK's open event map requires interface merging; a type alias cannot augment it.
  // eslint-disable-next-line @typescript-eslint/consistent-type-definitions
  interface AccountDataEvents {
    [PERSONAL_WORKSPACE_EVENT_TYPE]: WorkspaceDocument;
  }
}

export class MatrixWorkspaceGateway implements WorkspaceGateway {
  readonly #listeners = new Set<() => void>();
  #client: MatrixClient | null = null;
  #detach: (() => void) | null = null;

  constructor(private readonly source: MatrixClientSource) {}

  scope(): WorkspaceScope | null {
    const client = this.source.current();
    const accountId = client?.getUserId();
    const writerId = client?.getDeviceId();
    return accountId && writerId ? { accountId, writerId } : null;
  }

  async read(scope: WorkspaceScope) {
    const client = this.#scopedClient(scope);
    if (client === null) return err({ code: 'workspace.source_unavailable', retryable: true });
    try {
      const content: unknown = await client.getAccountDataFromServer(PERSONAL_WORKSPACE_EVENT_TYPE);
      return content === null ? ok(emptyWorkspace) : parseWorkspace(content);
    } catch {
      return err({ code: 'workspace.read_failed', retryable: true });
    }
  }

  async write(scope: WorkspaceScope, document: WorkspaceDocument) {
    const client = this.#scopedClient(scope);
    if (client === null) return err({ code: 'workspace.source_unavailable', retryable: true });
    try {
      await client.setAccountData(PERSONAL_WORKSPACE_EVENT_TYPE, document);
      return ok(undefined);
    } catch {
      return err({ code: 'workspace.write_failed', retryable: true });
    }
  }

  subscribe(listener: () => void): () => void {
    this.#listeners.add(listener);
    if (this.#listeners.size === 1) {
      this.#detach = this.source.subscribe(this.#onSource);
      this.#onSource();
    }
    return () => {
      this.#listeners.delete(listener);
      if (this.#listeners.size === 0) {
        this.#detach?.();
        this.#detach = null;
        this.#bind(null);
      }
    };
  }

  #scopedClient(scope: WorkspaceScope): MatrixClient | null {
    const current = this.scope();
    return current?.accountId === scope.accountId && current.writerId === scope.writerId
      ? this.source.current()
      : null;
  }

  readonly #onSource = (): void => {
    const client = this.source.current();
    if (this.#client === client) return;
    this.#bind(client);
    this.#notify();
  };
  readonly #onData = (event: MatrixEvent): void => {
    if (event.getType() === PERSONAL_WORKSPACE_EVENT_TYPE) this.#notify();
  };
  readonly #onSync = (state: SyncState, previous: SyncState | null): void => {
    if (
      state === SyncState.Prepared ||
      (state === SyncState.Syncing &&
        (previous === SyncState.Error || previous === SyncState.Reconnecting))
    )
      this.#notify();
  };

  #bind(client: MatrixClient | null): void {
    this.#client?.removeListener(ClientEvent.AccountData, this.#onData);
    this.#client?.removeListener(ClientEvent.Sync, this.#onSync);
    this.#client = client;
    client?.on(ClientEvent.AccountData, this.#onData);
    client?.on(ClientEvent.Sync, this.#onSync);
  }

  #notify(): void {
    for (const listener of this.#listeners) listener();
  }
}
