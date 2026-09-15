import { describe, expect, it } from 'vitest';
import {
  changeWorkspace,
  emptyWorkspace,
  mergeWorkspaces,
  parseWorkspace,
  workspaceIndex,
  type WorkspaceChange,
  type WorkspaceDocument,
} from './workspace-document';

const agent = '0198b601-77a1-7bb8-83eb-a8fe68c97e45';
const room = '!room:example.org';
function change(document: WorkspaceDocument, operation: WorkspaceChange, writer = 'desktop') {
  const result = changeWorkspace(document, operation, writer);
  if (!result.ok) throw new Error(result.error.code);
  return result.value;
}
function merge(left: WorkspaceDocument, right: WorkspaceDocument) {
  const result = mergeWorkspaces(left, right);
  if (!result.ok) throw new Error(result.error.code);
  return result.value;
}

describe('personal workspace', () => {
  it('keeps independent changes from two devices and converges in either order', () => {
    const desktop = change(emptyWorkspace, { kind: 'favorite', id: agent, value: true });
    const mobile = change(
      emptyWorkspace,
      { kind: 'tags', id: agent, value: ['Project One'] },
      'mobile',
    );
    const result = merge(desktop, mobile);
    expect(result).toEqual(merge(mobile, desktop));
    expect(workspaceIndex(result).favorites.has(agent)).toBe(true);
    expect(workspaceIndex(result).tags.get(agent)).toEqual(['Project One']);
    expect(merge(result, mobile)).toEqual(result);
  });
  it('does not rewind read progress when an older device has a higher logical clock', () => {
    const current = change(emptyWorkspace, {
      kind: 'read',
      id: room,
      value: { eventId: '$new', timestamp: 30 },
    });
    let old = change(emptyWorkspace, { kind: 'favorite', id: agent, value: true }, 'mobile');
    old = change(old, { kind: 'favorite', id: agent, value: false }, 'mobile');
    old = change(
      old,
      { kind: 'read', id: room, value: { eventId: '$old', timestamp: 10 } },
      'mobile',
    );
    expect(workspaceIndex(merge(current, old)).read.get(room)?.eventId).toBe('$new');
  });
  it('retains removal tombstones and resolves same-device concurrent edits deterministically', () => {
    const saved = change(emptyWorkspace, { kind: 'favorite', id: agent, value: true });
    const removed = change(saved, { kind: 'favorite', id: agent, value: false });
    expect(workspaceIndex(merge(removed, saved)).favorites.has(agent)).toBe(false);
    const left = change(emptyWorkspace, { kind: 'tags', id: agent, value: ['a'] });
    const right = change(emptyWorkspace, { kind: 'tags', id: agent, value: ['b'] });
    expect(merge(left, right)).toEqual(merge(right, left));
  });
  it('rejects duplicate registers, wrong value types and message bodies in account metadata', () => {
    const saved = change(emptyWorkspace, { kind: 'favorite', id: agent, value: true });
    expect(parseWorkspace({ schema: 1, entries: [...saved.entries, ...saved.entries] }).ok).toBe(
      false,
    );
    expect(
      parseWorkspace({
        schema: 1,
        entries: [{ kind: 'favorite', id: agent, value: 'yes', clock: 1, writerId: 'a' }],
      }).ok,
    ).toBe(false);
    expect(parseWorkspace({ ...saved, messageText: 'private body' }).ok).toBe(false);
  });
  it('makes an unchanged operation a no-op and keeps nested values immutable', () => {
    const saved = change(emptyWorkspace, { kind: 'tags', id: agent, value: ['project'] });
    expect(change(saved, { kind: 'tags', id: agent, value: ['project'] })).toBe(saved);
    expect(Object.isFrozen(saved.entries[0]?.value)).toBe(true);
  });
});
