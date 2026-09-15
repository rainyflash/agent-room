import { createContext, useContext, useEffect, useMemo, type ReactNode } from 'react';
import { ConversationWorkspaceStore } from '../application/conversation-workspace-store';
import { BrowserSubmissionIdFactory } from '@/features/messages/adapters/browser-submission-id-factory';
import type { MessagePublisher } from '@/features/messages/domain/publication';
import { BrowserConversationStorage } from '../domain/conversation-storage';
import { BrowserAttachmentStorage } from '../adapters/browser-attachment-storage';

const ConversationWorkspaceContext = createContext<ConversationWorkspaceStore | null>(null);
const submissionIds = new BrowserSubmissionIdFactory();

export function ConversationWorkspaceProvider(props: {
  readonly publisher: MessagePublisher;
  readonly scope: string | null;
  readonly children: ReactNode;
}) {
  const inherited = useContext(ConversationWorkspaceContext);
  return inherited === null ? <OwnedConversationWorkspace {...props} /> : <>{props.children}</>;
}

function OwnedConversationWorkspace({
  publisher,
  scope,
  children,
}: {
  readonly publisher: MessagePublisher;
  readonly scope: string | null;
  readonly children: ReactNode;
}) {
  const store = useMemo(
    () =>
      new ConversationWorkspaceStore(
        publisher,
        submissionIds,
        scope === null
          ? undefined
          : new BrowserConversationStorage(
              {
                getItem: (key) => window.localStorage.getItem(key),
                setItem: (key, value) => {
                  window.localStorage.setItem(key, value);
                },
                removeItem: (key) => {
                  window.localStorage.removeItem(key);
                },
              },
              scope,
            ),
        scope === null ? undefined : new BrowserAttachmentStorage(scope),
      ),
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
