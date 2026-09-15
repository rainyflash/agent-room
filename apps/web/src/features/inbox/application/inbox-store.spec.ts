import { describe, expect, it, vi } from 'vitest';
import { PersonalWorkspaceStore } from '@/features/personal-workspace/application/personal-workspace-store';
import { emptyWorkspace } from '@/features/personal-workspace/domain/workspace-document';
import { conversationFixture } from '@/features/conversation/testing/conversation-fixture';
import { err, ok, type Result } from '@/shared/result';
import { InboxStore } from './inbox-store';
import type { InboxFailure, InboxIndex, InboxIndexGateway } from '../domain/inbox';

function setup() {
  let account: string | null = '@a:example.org';
  const listeners = new Set<() => void>();
  const subscribe = (listener: () => void) => {
    listeners.add(listener);
    return () => {
      listeners.delete(listener);
    };
  };
  const personal = new PersonalWorkspaceStore(
    {
      scope: () => (account ? { accountId: account, writerId: 'test' } : null),
      read: () => Promise.resolve(ok(emptyWorkspace)),
      write: () => Promise.resolve(ok(undefined)),
      subscribe,
    },
    { read: () => ok(emptyWorkspace), write: () => ok(undefined) },
  );
  const index = (accountId: string): InboxIndex => ({
    accountId,
    rooms: [{ catalogId: 'catalog', roomId: '!room:example.org', name: 'Project', direct: true }],
    handoffs: [],
    limited: false,
  });
  const gateway: InboxIndexGateway = {
    read: vi.fn<InboxIndexGateway['read']>((id) => Promise.resolve(ok(index(id)))),
  };
  const messages = {
    read: () =>
      ok({
        roomId: '!room:example.org',
        messages: [conversationFixture('message')],
        observedAtUnixMs: 0,
        readOnlyFederatedEvents: [],
      }),
    subscribe: (_roomId: string, listener: () => void) => subscribe(listener),
  };
  const store = new InboxStore(
    gateway,
    { accountId: () => account, isJoined: () => true, isIgnored: () => false, subscribe },
    messages,
    personal,
  );
  return {
    store,
    gateway,
    index,
    changeAccount: (value: string | null) => {
      account = value;
      for (const listener of listeners) listener();
    },
  };
}
describe('inbox account lifecycle', () => {
  it('clears private results at logout and ignores old in-flight responses after account switch', async () => {
    const fixture = setup();
    let complete: (value: Result<InboxIndex, InboxFailure>) => void = () => undefined;
    vi.mocked(fixture.gateway.read).mockImplementationOnce(
      () =>
        new Promise((resolve) => {
          complete = resolve;
        }),
    );
    const stop = fixture.store.subscribe(() => undefined);
    fixture.changeAccount('@b:example.org');
    await vi.waitFor(() => {
      expect(fixture.store.getSnapshot().status).toBe('ready');
    });
    complete(ok(fixture.index('@a:example.org')));
    await Promise.resolve();
    expect(fixture.store.getSnapshot().accountId).toBe('@b:example.org');
    fixture.changeAccount(null);
    expect(fixture.store.getSnapshot()).toMatchObject({
      accountId: null,
      status: 'signed_out',
      items: [],
    });
    stop();
  });
  it('keeps current-account results with a visible failure and supports retry', async () => {
    const fixture = setup();
    const stop = fixture.store.subscribe(() => undefined);
    await vi.waitFor(() => {
      expect(fixture.store.getSnapshot().items).toHaveLength(1);
    });
    vi.mocked(fixture.gateway.read).mockResolvedValueOnce(
      err({ code: 'inbox.offline', retryable: true }),
    );
    fixture.store.refresh();
    await vi.waitFor(() => {
      expect(fixture.store.getSnapshot().status).toBe('failed');
    });
    expect(fixture.store.getSnapshot().items).toHaveLength(1);
    fixture.store.refresh();
    await vi.waitFor(() => {
      expect(fixture.store.getSnapshot().status).toBe('ready');
    });
    stop();
  });
});
