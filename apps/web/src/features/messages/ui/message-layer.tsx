import { AnimatePresence } from 'motion/react';
import { useEffect, useMemo, useSyncExternalStore } from 'react';

import { useAppServices } from '@/app/app-services';
import { MessageRoomStore } from '@/features/messages/application/message-room-store';
import { ContentInspector } from '@/features/messages/ui/content-inspector';
import { MessageComposer } from '@/features/messages/ui/message-composer';
import { projectMessageSignals } from '@/features/signals/adapters/message-signal-projector';
import type { SignalAction } from '@/features/signals/domain/signal';
import { SignalDock } from '@/features/signals/ui/signal-dock';

export type MessageLayerProps = {
  readonly onSelectedMessageChange: (messageId: string | null) => void;
  readonly roomId: string;
  readonly roomName: string;
  readonly selectedMessageId: string | null;
};

export function MessageLayer({
  onSelectedMessageChange,
  roomId,
  roomName,
  selectedMessageId,
}: MessageLayerProps) {
  const { content, contentVerifier, messagePublisher, messages } = useAppServices();
  const store = useMemo(() => new MessageRoomStore(messages, roomId), [messages, roomId]);
  const state = useSyncExternalStore(store.subscribe, store.getSnapshot, store.getSnapshot);
  const projectedMessages = state.kind === 'ready' ? state.room.messages : [];
  const projectedSignals = useMemo(
    () => projectMessageSignals(projectedMessages, 'room'),
    [projectedMessages],
  );
  const selectedMessage =
    projectedMessages.find((message) => message.messageId === selectedMessageId) ?? null;
  const selectedSignalId = selectedMessageId === null ? null : `message:${selectedMessageId}`;

  const handleSignalAction = (action: SignalAction): void => {
    if (action.kind === 'open_message') {
      onSelectedMessageChange(action.messageId);
    }
  };

  useEffect(() => {
    if (selectedMessageId !== null && state.kind === 'ready' && selectedMessage === null) {
      onSelectedMessageChange(null);
    }
  }, [onSelectedMessageChange, selectedMessage, selectedMessageId, state.kind]);

  return (
    <>
      <div className="message-hub">
        {state.kind === 'loading' ? null : (
          <SignalDock
            onAction={handleSignalAction}
            onRetry={store.retry}
            selectedSignalId={selectedSignalId}
            signals={projectedSignals}
            state={state.kind === 'ready' ? 'ready' : 'failed'}
          />
        )}
        <MessageComposer
          key={roomId}
          publisher={messagePublisher}
          roomId={roomId}
          roomName={roomName}
        />
      </div>
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
