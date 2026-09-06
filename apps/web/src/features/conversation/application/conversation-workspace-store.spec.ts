import { afterEach, describe, expect, it, vi } from 'vitest';
import { ConversationWorkspaceStore } from './conversation-workspace-store';
import type {
  MessagePublicationRequest,
  MessagePublicationResult,
  MessagePublisher,
} from '@/features/messages/domain/publication';
import { ok } from '@/shared/result';

const submissionId = '01990d9e-8400-7000-8000-000000000003';
const releases: (() => void)[] = [];
afterEach(async () => {
  for (const release of releases.splice(0)) release();
  await Promise.resolve();
});

function fixture() {
  const publish = vi.fn((request: MessagePublicationRequest): Promise<MessagePublicationResult> =>
    Promise.resolve(published(request.submissionId)),
  );
  const reconcile = vi.fn((id: string): Promise<MessagePublicationResult> =>
    Promise.resolve(published(id)),
  );
  const publisher: MessagePublisher = {
    publish,
    reconcile,
    resolveIdentity: () =>
      Promise.resolve(
        ok({
          kind: 'human',
          matrixUserId: '@owner:room.test',
          displayName: 'Owner',
          principalId: submissionId,
          source: 'matrix_human_session',
        }),
      ),
  };
  const ids = { next: vi.fn(() => submissionId) };
  const workspace = new ConversationWorkspaceStore(publisher, ids);
  const release = workspace.retain();
  releases.push(release);
  return { workspace, publish, reconcile, ids, release };
}

function published(id: string): MessagePublicationResult {
  return ok({ kind: 'published', submissionId: id, matrixEventId: '$published', reused: false });
}

async function open(workspace: ConversationWorkspaceStore, roomId: string) {
  const session = workspace.room(roomId);
  const detach = session.subscribe(() => undefined);
  releases.push(detach);
  await vi.waitFor(() => {
    expect(session.getSnapshot().publication.matches('closed')).toBe(false);
  });
  return { session, detach };
}

describe('工作区对话生命周期', () => {
  it('卸载面板保留私聊草稿、提及，各个房间相互隔离', async () => {
    const { workspace } = fixture();
    const a = await open(workspace, '!a:room.test');
    await vi.waitFor(() => {
      expect(a.session.editable).toBe(true);
    });
    a.session.changeText('Draft A');
    a.session.mention('@agent:room.test');
    a.detach();
    const b = await open(workspace, '!b:room.test');
    await vi.waitFor(() => {
      expect(b.session.editable).toBe(true);
    });
    expect(b.session.getSnapshot().text).toBe('');
    b.session.changeText('Draft B');
    const reopened = await open(workspace, '!a:room.test');
    expect(reopened.session.getSnapshot()).toMatchObject({
      text: 'Draft A',
      mentions: ['@agent:room.test'],
    });
    expect(b.session.getSnapshot().text).toBe('Draft B');
  });

  it('发送中离开私聊，后台确认后只清除该会话草稿', async () => {
    const { workspace, publish, ids } = fixture();
    const pending = Promise.withResolvers<MessagePublicationResult>();
    publish.mockReturnValueOnce(pending.promise);
    const a = await open(workspace, '!a:room.test');
    await vi.waitFor(() => {
      expect(a.session.editable).toBe(true);
    });
    a.session.changeText('Send once');
    a.session.submit();
    a.detach();
    const b = await open(workspace, '!b:room.test');
    await vi.waitFor(() => {
      expect(b.session.editable).toBe(true);
    });
    b.session.changeText('Leave this draft alone');
    pending.resolve(published(submissionId));
    await vi.waitFor(() => {
      expect(a.session.getSnapshot().publication.matches('published')).toBe(true);
    });
    const reopened = await open(workspace, '!a:room.test');
    expect(reopened.session.getSnapshot().text).toBe('');
    expect(b.session.getSnapshot().text).toBe('Leave this draft alone');
    expect(publish).toHaveBeenCalledOnce();
    expect(ids.next).toHaveBeenCalledOnce();
  });

  it('结果未知时重开私聊仍锁定发送，并使用原提交标识确认', async () => {
    const { workspace, publish, reconcile, ids } = fixture();
    publish.mockResolvedValueOnce(
      ok({ kind: 'pending_reconciliation', submissionId, transactionId: 'pending' }),
    );
    const a = await open(workspace, '!a:room.test');
    await vi.waitFor(() => {
      expect(a.session.editable).toBe(true);
    });
    a.session.changeText('Do not duplicate');
    a.session.submit();
    await vi.waitFor(() => {
      expect(a.session.getSnapshot().publication.matches('unknown')).toBe(true);
    });
    a.detach();
    const reopened = await open(workspace, '!a:room.test');
    expect(reopened.session.editable).toBe(false);
    expect(reopened.session.getSnapshot().text).toBe('Do not duplicate');
    reopened.session.submit();
    expect(publish).toHaveBeenCalledOnce();
    expect(ids.next).toHaveBeenCalledOnce();
    reopened.session.reconcile();
    await vi.waitFor(() => {
      expect(reopened.session.getSnapshot().publication.matches('published')).toBe(true);
    });
    expect(reconcile).toHaveBeenCalledExactlyOnceWith(submissionId);
    expect(reopened.session.getSnapshot().text).toBe('');
  });

  it('StrictMode 同轮重连不清草稿，退出工作区后释放会话', async () => {
    const { workspace, release } = fixture();
    const a = await open(workspace, '!a:room.test');
    await vi.waitFor(() => {
      expect(a.session.editable).toBe(true);
    });
    a.session.changeText('Strict mode draft');
    a.detach();
    release();
    const retained = workspace.retain();
    releases.push(retained);
    await Promise.resolve();
    expect(workspace.room('!a:room.test').getSnapshot().text).toBe('Strict mode draft');
    retained();
    await Promise.resolve();
    expect(workspace.room('!a:room.test').getSnapshot().text).toBe('');
  });
});
