import type { MessageGateway } from '@/features/messages/domain/message';
import type { PersonalWorkspaceStore } from '@/features/personal-workspace/application/personal-workspace-store';
import {
  projectInbox,
  type InboxIndex,
  type InboxIndexGateway,
  type InboxMatrixSource,
  type InboxItem,
} from '../domain/inbox';

export type InboxSnapshot = {
  readonly accountId: string | null;
  readonly items: readonly InboxItem[];
  readonly limited: boolean;
  readonly unavailableRooms: number;
  readonly status: 'loading' | 'ready' | 'failed' | 'signed_out';
};
const initial: InboxSnapshot = Object.freeze({
  accountId: null,
  items: [],
  limited: false,
  unavailableRooms: 0,
  status: 'signed_out',
});

export class InboxStore {
  #snapshot = initial;
  #index: InboxIndex | null = null;
  #generation = 0;
  #loading = false;
  #metadataFailed = false;
  #timer: ReturnType<typeof setInterval> | null = null;
  #scheduled: ReturnType<typeof setTimeout> | null = null;
  #detach: readonly (() => void)[] = [];
  readonly #listeners = new Set<() => void>();
  constructor(
    private readonly gateway: InboxIndexGateway,
    private readonly source: InboxMatrixSource,
    private readonly messages: MessageGateway,
    private readonly personal: PersonalWorkspaceStore,
  ) {}
  readonly getSnapshot = () => this.#snapshot;
  readonly subscribe = (listener: () => void): (() => void) => {
    this.#listeners.add(listener);
    if (this.#listeners.size === 1) {
      this.#detach = [
        this.source.subscribe(this.#sourceChanged),
        this.personal.subscribe(this.#sourceChanged),
      ];
      this.#sourceChanged();
      this.refresh();
      this.#timer = setInterval(this.refresh, 60000);
    }
    return () => {
      this.#listeners.delete(listener);
      if (this.#listeners.size === 0) {
        for (const detach of this.#detach) detach();
        this.#detach = [];
        if (this.#timer !== null) clearInterval(this.#timer);
        if (this.#scheduled !== null) clearTimeout(this.#scheduled);
        this.#timer = null;
        this.#scheduled = null;
        this.#generation += 1;
        this.#loading = false;
      }
    };
  };
  readonly refresh = (): void => {
    const account = this.source.accountId();
    if (!account || this.#loading || this.#listeners.size === 0) return;
    this.#loading = true;
    const generation = this.#generation;
    void this.gateway.read(account).then(
      (result) => {
        if (generation !== this.#generation || account !== this.source.accountId()) return;
        this.#loading = false;
        if (!result.ok) {
          this.#metadataFailed = true;
          this.#publish({ ...this.#snapshot, status: 'failed' });
          return;
        }
        this.#metadataFailed = false;
        this.#index = result.value;
        this.#project();
      },
      () => {
        if (generation !== this.#generation || account !== this.source.accountId()) return;
        this.#loading = false;
        this.#metadataFailed = true;
        this.#publish({ ...this.#snapshot, status: 'failed' });
      },
    );
  };
  readonly #sourceChanged = (): void => {
    const account = this.source.accountId();
    if (account !== this.#snapshot.accountId) {
      this.#generation += 1;
      this.#loading = false;
      this.#index = null;
      this.#metadataFailed = false;
      this.#publish({ ...initial, accountId: account, status: account ? 'loading' : 'signed_out' });
      this.refresh();
    }
    if (this.#scheduled === null)
      this.#scheduled = setTimeout(() => {
        this.#scheduled = null;
        this.#project();
      }, 150);
  };
  #project(): void {
    const personal = this.personal.getSnapshot();
    if (!this.#index || personal.accountId !== this.#index.accountId) return;
    const projected = projectInbox(this.#index, this.messages, this.source, personal.index);
    this.#publish({
      items: projected.items,
      unavailableRooms: projected.unavailableRooms,
      limited: this.#index.limited,
      accountId: this.#index.accountId,
      status: this.#metadataFailed ? 'failed' : 'ready',
    });
    const recent = [...projected.contacts].filter(
      ([id, timestamp]) => timestamp > (personal.index.recent.get(id) ?? 0),
    );
    if (recent.length > 0 && personal.status !== 'failed')
      this.personal.changeMany(recent.map(([id, value]) => ({ kind: 'recent', id, value })));
  }
  #publish(snapshot: InboxSnapshot): void {
    this.#snapshot = Object.freeze(snapshot);
    for (const listener of this.#listeners) listener();
  }
}
