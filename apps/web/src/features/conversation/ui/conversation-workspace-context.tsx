import { createContext, useContext, useEffect, useMemo, type ReactNode } from 'react';
import { ConversationWorkspaceStore } from '../application/conversation-workspace-store';
import { BrowserSubmissionIdFactory } from '@/features/messages/adapters/browser-submission-id-factory';
import type { MessagePublisher } from '@/features/messages/domain/publication';

const ConversationWorkspaceContext = createContext<ConversationWorkspaceStore | null>(null);
const submissionIds = new BrowserSubmissionIdFactory();

export function ConversationWorkspaceProvider({
  publisher,
  scope,
  children,
}: {
  readonly publisher: MessagePublisher;
  readonly scope: string | null;
  readonly children: ReactNode;
}) {
  const store = useMemo(
    () => new ConversationWorkspaceStore(publisher, submissionIds),
    [publisher, scope],
  );
  useEffect(store.retain, [store]);
  return (
    <ConversationWorkspaceContext.Provider value={store}>
      {children}
    </ConversationWorkspaceContext.Provider>
  );
}

export function useConversationWorkspace(): ConversationWorkspaceStore | null {
  return useContext(ConversationWorkspaceContext);
}
