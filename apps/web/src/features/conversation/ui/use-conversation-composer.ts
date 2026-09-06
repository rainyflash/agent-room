import { useEffect, useMemo, useSyncExternalStore } from 'react';
import { ConversationWorkspaceStore } from '../application/conversation-workspace-store';
import { validConversation } from '../domain/conversation';
import { useConversationWorkspace } from './conversation-workspace-context';
import {
  BrowserSubmissionIdFactory,
  type MessageSubmissionIdFactory,
} from '@/features/messages/adapters/browser-submission-id-factory';
import type { MessagePublisher } from '@/features/messages/domain/publication';

const browserIds = new BrowserSubmissionIdFactory();

export function useConversationComposer(
  publisher: MessagePublisher,
  roomId: string,
  submissionIds: MessageSubmissionIdFactory = browserIds,
) {
  const shared = useConversationWorkspace();
  const workspace = useMemo(
    () => shared ?? new ConversationWorkspaceStore(publisher, submissionIds),
    [shared, publisher, submissionIds],
  );
  useEffect(workspace.retain, [workspace]);
  const session = useMemo(() => workspace.room(roomId), [workspace, roomId]);
  const snapshot = useSyncExternalStore(
    session.subscribe,
    session.getSnapshot,
    session.getSnapshot,
  );
  return {
    ...snapshot,
    editable: session.editable,
    valid: validConversation(snapshot),
    changeText: session.changeText,
    mention: session.mention,
    respond: session.respond,
    removeMention: session.removeMention,
    cancelReply: session.cancelReply,
    submit: session.submit,
    retry: session.retry,
    reconcile: session.reconcile,
    edit: session.edit,
    retryIdentity: session.retryIdentity,
  };
}
