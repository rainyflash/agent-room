import { describe, expect, it, vi } from 'vitest';
import { ok, err } from '@/shared/result';
import { BrowserWorkspaceCache } from '../adapters/browser-workspace-cache';
import {
  changeWorkspace,
  emptyWorkspace,
  workspaceIndex,
  type WorkspaceDocument,
} from '../domain/workspace-document';
import type { WorkspaceGateway, WorkspaceScope } from '../domain/workspace-gateway';
import { PersonalWorkspaceStore } from './personal-workspace-store';

const agent = '0198b601-77a1-7bb8-83eb-a8fe68c97e45';
function setup() {
  const values = new Map<string, string>();
  const cache = new BrowserWorkspaceCache({
    getItem: (key) => values.get(key) ?? null,
    setItem: (key, value) => {
      values.set(key, value);
    },
  });
  let scope: WorkspaceScope | null = { accountId: '@a:example.org', writerId: 'desktop' };
  let listener: () => void = () => undefined;
  let remote = emptyWorkspace;
  const gateway: WorkspaceGateway = {
    scope: () => scope,
    read: vi.fn<WorkspaceGateway['read']>(() => Promise.resolve(ok(remote))),
    write: vi.fn<WorkspaceGateway['write']>((_scope, document) => {
      remote = document;
      return Promise.resolve(ok(undefined));
    }),
    subscribe: (next) => {
      listener = next;
      return () => {
        listener = () => undefined;
      };
    },
  };
  return {
    cache,
    gateway,
    changeScope: (next: WorkspaceScope | null) => {
      scope = next;
      listener();
    },
    setRemote: (next: WorkspaceDocument) => {
      remote = next;
      listener();
    },
  };
}

describe('personal workspace synchronization', () => {
  it('continues syncing the new account after the old account request throws', async () => {
    const fixture = setup();
    let fail: (reason: Error) => void = () => undefined;
    vi.mocked(fixture.gateway.read).mockImplementationOnce(
      () =>
        new Promise((_resolve, reject) => {
          fail = reject;
        }),
    );
    const store = new PersonalWorkspaceStore(fixture.gateway, fixture.cache);
    const stop = store.subscribe(() => undefined);
    fixture.changeScope({ accountId: '@b:example.org', writerId: 'other' });
    fail(new Error('old connection failed'));
    await vi.waitFor(() => {
      expect(store.getSnapshot().status).toBe('synced');
    });
    expect(store.getSnapshot().accountId).toBe('@b:example.org');
    expect(store.getSnapshot().index.favorites.size).toBe(0);
    stop();
  });
  it('undoes only the matching favorite revision and never another account', () => {
    const fixture = setup();
    const store = new PersonalWorkspaceStore(fixture.gateway, fixture.cache);
    const stop = store.subscribe(() => undefined);
    const first = store.toggleFavorite(agent);
    if (!first.ok) throw new Error(first.error.code);
    expect(store.getSnapshot().index.favorites.has(agent)).toBe(true);
    expect(store.undoFavorite(first.value).ok).toBe(true);
    expect(store.getSnapshot().index.favorites.has(agent)).toBe(false);
    expect(store.undoFavorite(first.value).ok).toBe(false);
    const second = store.toggleFavorite(agent);
    if (!second.ok) throw new Error(second.error.code);
    fixture.changeScope({ accountId: '@b:example.org', writerId: 'other' });
    expect(store.undoFavorite(second.value).ok).toBe(false);
    expect(
      store.changeForAccount('@a:example.org', { kind: 'favorite', id: agent, value: true }).ok,
    ).toBe(false);
    expect(store.getSnapshot().index.favorites.size).toBe(0);
    stop();
  });
  it('does not apply part of an invalid batch', () => {
    const fixture = setup();
    const store = new PersonalWorkspaceStore(fixture.gateway, fixture.cache);
    const stop = store.subscribe(() => undefined);
    const changed = store.changeMany([
      { kind: 'favorite', id: agent, value: true },
      { kind: 'tags', id: agent, value: ['x'.repeat(33)] },
    ]);
    expect(changed.ok).toBe(false);
    expect(store.getSnapshot().index.favorites.size).toBe(0);
    stop();
  });
  it('keeps offline changes after restarting and merges remote edits when reconnecting', async () => {
    const fixture = setup();
    vi.mocked(fixture.gateway.read).mockResolvedValue(err({ code: 'offline', retryable: true }));
    const first = new PersonalWorkspaceStore(fixture.gateway, fixture.cache);
    const detach = first.subscribe(() => undefined);
    expect(first.change({ kind: 'favorite', id: agent, value: true }).ok).toBe(true);
    await vi.waitFor(() => {
      expect(first.getSnapshot().status).toBe('failed');
    });
    detach();
    const remote = changeWorkspace(
      emptyWorkspace,
      { kind: 'tags', id: agent, value: ['shared'] },
      'phone',
    );
    if (!remote.ok) throw new Error(remote.error.code);
    vi.mocked(fixture.gateway.read).mockImplementation(() => Promise.resolve(ok(remote.value)));
    vi.mocked(fixture.gateway.write).mockImplementation((_scope, document) => {
      vi.mocked(fixture.gateway.read).mockResolvedValue(ok(document));
      return Promise.resolve(ok(undefined));
    });
    const restored = new PersonalWorkspaceStore(fixture.gateway, fixture.cache);
    const stop = restored.subscribe(() => undefined);
    await vi.waitFor(() => {
      expect(restored.getSnapshot().status).toBe('synced');
    });
    expect(restored.getSnapshot().index.favorites.has(agent)).toBe(true);
    expect(restored.getSnapshot().index.tags.get(agent)).toEqual(['shared']);
    stop();
  });
  it('does not lose a second edit made while a previous write is in flight', async () => {
    const fixture = setup();
    let release: () => void = () => undefined;
    const waiting = new Promise<void>((resolve) => {
      release = resolve;
    });
    vi.mocked(fixture.gateway.write).mockImplementationOnce(async (_scope, document) => {
      await waiting;
      fixture.setRemote(document);
      return ok(undefined);
    });
    const store = new PersonalWorkspaceStore(fixture.gateway, fixture.cache);
    const stop = store.subscribe(() => undefined);
    await vi.waitFor(() => {
      expect(store.getSnapshot().status).toBe('synced');
    });
    store.change({ kind: 'favorite', id: agent, value: true });
    await vi.waitFor(() => {
      expect(fixture.gateway.write).toHaveBeenCalledOnce();
    });
    store.change({ kind: 'tags', id: agent, value: ['while writing'] });
    release();
    await vi.waitFor(() => {
      expect(store.getSnapshot().status).toBe('synced');
    });
    expect(store.getSnapshot().index.tags.get(agent)).toEqual(['while writing']);
    const writes = vi.mocked(fixture.gateway.write).mock.calls;
    const last = writes.at(-1)?.[1];
    expect(last && workspaceIndex(last).tags.get(agent)).toEqual(['while writing']);
    stop();
  });
  it('clears private state immediately on account change and ignores an old response', async () => {
    const fixture = setup();
    const old = changeWorkspace(
      emptyWorkspace,
      { kind: 'favorite', id: agent, value: true },
      'old',
    );
    if (!old.ok) throw new Error(old.error.code);
    let complete: (document: WorkspaceDocument) => void = () => undefined;
    vi.mocked(fixture.gateway.read).mockImplementationOnce(
      () =>
        new Promise((resolve) => {
          complete = (document) => {
            resolve(ok(document));
          };
        }),
    );
    const store = new PersonalWorkspaceStore(fixture.gateway, fixture.cache);
    const stop = store.subscribe(() => undefined);
    fixture.changeScope({ accountId: '@b:example.org', writerId: 'phone' });
    expect(store.getSnapshot().accountId).toBe('@b:example.org');
    expect(store.getSnapshot().index.favorites.size).toBe(0);
    complete(old.value);
    await vi.waitFor(() => {
      expect(store.getSnapshot().status).toBe('synced');
    });
    expect(store.getSnapshot().index.favorites.size).toBe(0);
    expect(fixture.gateway.write).not.toHaveBeenCalled();
    stop();
  });
  it('reports storage failures instead of claiming a preference was saved', () => {
    const fixture = setup();
    const cache = new BrowserWorkspaceCache({
      getItem: () => null,
      setItem: () => {
        throw new Error('quota');
      },
    });
    const store = new PersonalWorkspaceStore(fixture.gateway, cache);
    const stop = store.subscribe(() => undefined);
    expect(store.change({ kind: 'favorite', id: agent, value: true })).toEqual(
      err({ code: 'workspace.cache_write_failed', retryable: true }),
    );
    expect(store.getSnapshot().index.favorites.has(agent)).toBe(false);
    stop();
  });
});
