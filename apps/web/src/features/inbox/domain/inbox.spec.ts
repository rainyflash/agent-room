import { describe, expect, it } from 'vitest';
import { conversationFixture } from '@/features/conversation/testing/conversation-fixture';
import type { AgentMessageActor } from '@/features/messages/domain/message';
import {
  emptyWorkspace,
  workspaceIndex,
} from '@/features/personal-workspace/domain/workspace-document';
import { ok } from '@/shared/result';
import {
  notificationAllowed,
  projectInbox,
  type InboxIndex,
  type InboxMatrixSource,
} from './inbox';

const self = '@me:example.org';
const roomId = '!room:example.org';
const index: InboxIndex = {
  accountId: self,
  rooms: [{ catalogId: 'catalog', roomId, name: 'Project', direct: false }],
  handoffs: [],
  limited: false,
};
const source: InboxMatrixSource = {
  accountId: () => self,
  isJoined: () => true,
  isIgnored: () => false,
  subscribe: () => () => undefined,
};

describe('personal inbox', () => {
  it('projects mentions and replies only once and never exposes another account or a left room', () => {
    const mine = conversationFixture('mine', {
      actor: {
        kind: 'human',
        displayName: 'Me',
        matrixUserId: self,
        principalId: 'self',
        provenance: 'human',
      },
    });
    const reply = conversationFixture('reply', {
      relation: { kind: 'reply', targetMessageId: 'mine' },
    });
    const base = conversationFixture('mention');
    const mention = conversationFixture('mention', {
      preview: base.preview
        ? { ...base.preview, conversation: { text: 'Please look', mentions: [self] } }
        : null,
      relation: { kind: 'reply', targetMessageId: 'mine' },
    });
    const messages = {
      read: () =>
        ok({
          roomId,
          messages: [
            mine,
            reply,
            mention,
            mention,
            conversationFixture('noise'),
            conversationFixture('removed', { lifecycle: 'redacted' }),
          ],
          observedAtUnixMs: 0,
          readOnlyFederatedEvents: [],
        }),
      subscribe: () => () => undefined,
    };
    const prefs = workspaceIndex(emptyWorkspace);
    expect(
      projectInbox(index, messages, source, prefs)
        .items.map((item) => item.kind)
        .toSorted(),
    ).toEqual(['mention', 'reply']);
    expect(
      projectInbox(index, messages, { ...source, accountId: () => '@other:example.org' }, prefs)
        .items,
    ).toEqual([]);
    expect(
      projectInbox(index, messages, { ...source, isJoined: () => false }, prefs).items,
    ).toEqual([]);
  });
  it('retains muted items in the inbox, suppresses reminders and respects synchronized read progress', () => {
    const message = conversationFixture('direct');
    const messages = {
      read: () =>
        ok({ roomId, messages: [message], observedAtUnixMs: 0, readOnlyFederatedEvents: [] }),
      subscribe: () => () => undefined,
    };
    const direct: InboxIndex = {
      ...index,
      rooms: index.rooms.map((room) => ({ ...room, direct: true })),
    };
    const prefs = workspaceIndex(emptyWorkspace);
    prefs.muted.add(roomId);
    let item = projectInbox(direct, messages, source, prefs).items[0];
    if (!item) throw new Error('Missing inbox item');
    expect(item.read).toBe(false);
    expect(notificationAllowed(item, prefs, Date.now())).toBe(false);
    prefs.read.set(roomId, { timestamp: message.serverTimestamp, eventId: message.matrixEventId });
    item = projectInbox(direct, messages, source, prefs).items[0];
    expect(item?.read).toBe(true);
  });
  it('records actual exchanges with agents, not unrelated room activity or ignored senders', () => {
    const agent: AgentMessageActor = {
      kind: 'agent',
      agentId: 'agent',
      instanceId: 'instance',
      displayName: 'Agent',
      matrixUserId: '@agent:example.org',
      provenance: 'autonomous_agent',
    };
    const agentMessage = conversationFixture('agent-message', { actor: agent, serverTimestamp: 1 });
    const noise = conversationFixture('noise', {
      actor: { ...agent, agentId: 'noise', matrixUserId: '@noise:example.org' },
      serverTimestamp: 100,
    });
    const mine = conversationFixture('mine', {
      actor: {
        kind: 'human',
        principalId: 'me',
        matrixUserId: self,
        displayName: 'Me',
        provenance: 'human',
      },
      serverTimestamp: 10,
      relation: { kind: 'reply', targetMessageId: agentMessage.messageId },
    });
    const gateway = {
      read: () =>
        ok({
          roomId,
          messages: [agentMessage, noise, mine],
          observedAtUnixMs: 0,
          readOnlyFederatedEvents: [],
        }),
      subscribe: () => () => undefined,
    };
    const prefs = workspaceIndex(emptyWorkspace);
    const result = projectInbox(index, gateway, source, prefs);
    expect([...result.contacts]).toEqual([['agent', 10]]);
    expect(result.items).toEqual([]);
    expect(
      projectInbox(
        index,
        gateway,
        { ...source, isIgnored: (id) => id === agent.matrixUserId },
        prefs,
      ).contacts.size,
    ).toBe(0);
  });
});
