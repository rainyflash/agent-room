import { AnimatePresence } from 'motion/react';
import { useEffect, useMemo, useSyncExternalStore } from 'react';

import { useAppServices } from '@/app/app-services';
import { MessageRoomStore } from '@/features/messages/application/message-room-store';
import { ContentInspector } from '@/features/messages/ui/content-inspector';
import { MessageSignalDock } from '@/features/messages/ui/message-signal-dock';

export type MessageLayerProps = {
  readonly onSelectedMessageChange: (messageId: string | null) => void;
  readonly roomId: string;
  readonly selectedMessageId: string | null;
};

export function MessageLayer({
  onSelectedMessageChange,
  roomId,
  selectedMessageId,
}: MessageLayerProps) {
  const { content, contentVerifier, messages } = useAppServices();
  const store = useMemo(() => new MessageRoomStore(messages, roomId), [messages, roomId]);
  const state = useSyncExternalStore(store.subscribe, store.getSnapshot, store.getSnapshot);
  const projectedMessages = state.kind === 'ready' ? state.room.messages : [];
  const selectedMessage =
    projectedMessages.find((message) => message.messageId === selectedMessageId) ?? null;

  useEffect(() => {
    if (selectedMessageId !== null && state.kind === 'ready' && selectedMessage === null) {
      onSelectedMessageChange(null);
    }
  }, [onSelectedMessageChange, selectedMessage, selectedMessageId, state.kind]);

  return (
    <>
      {state.kind === 'loading' ? null : (
        <MessageSignalDock
          messages={projectedMessages}
          onRetry={store.retry}
          onSelectMessage={onSelectedMessageChange}
          selectedMessageId={selectedMessageId}
          state={state.kind === 'ready' ? 'ready' : 'failed'}
        />
      )}
      <AnimatePresence>
        {selectedMessage === null ? null : (
          <ContentInspector
            contentGateway={content}
            contentVerifier={contentVerifier}
            key={selectedMessage.messageId}
            message={selectedMessage}
            onClose={() => {
              onSelectedMessageChange(null);
            }}
          />
        )}
      </AnimatePresence>
    </>
  );
}
