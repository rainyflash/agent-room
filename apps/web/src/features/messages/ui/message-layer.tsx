import { ConversationPanel } from '@/features/conversation/ui/conversation-panel';
import type { ConversationParticipant } from '@/features/conversation/domain/conversation';
import { AnimatePresence } from 'motion/react';
import { useEffect, useMemo, useRef, useSyncExternalStore } from 'react';
import { useTranslation } from 'react-i18next';

import { useAppServices } from '@/app/app-services';
import { MessageRoomStore } from '@/features/messages/application/message-room-store';
import type { ReadOnlyFederatedEvent } from '@/features/messages/domain/message';
import { ContentInspector } from '@/features/messages/ui/content-inspector';
import { MessageComposer } from '@/features/messages/ui/message-composer';
import { projectMessageSignals } from '@/features/signals/adapters/message-signal-projector';
import type { SignalAction } from '@/features/signals/domain/signal';
import { SignalDock } from '@/features/signals/ui/signal-dock';

export type MessageLayerProps = {
  readonly participants?: readonly ConversationParticipant[];
  readonly catalogId: string;
  readonly onLatestDisplayed?: (matrixEventId: string) => void;
  readonly onSelectedMessageChange: (messageId: string | null) => void;
  readonly roomId: string;
  readonly roomName: string;
  readonly selectedMessageId: string | null;
  readonly variant?: 'direct' | 'room';
  readonly writesAllowed?: boolean;
};

export function MessageLayer({
  participants,
  catalogId,
  onLatestDisplayed,
  onSelectedMessageChange,
  roomId,
  roomName,
  selectedMessageId,
  variant = 'room',
  writesAllowed = true,
}: MessageLayerProps) {
  const {
    content,
    contentVerifier,
    handoffs,
    messagePublisher,
    messages,
    messageTranslation,
    moderation,
    telemetry,
  } = useAppServices();
  const store = useMemo(() => new MessageRoomStore(messages, roomId), [messages, roomId]);
  const state = useSyncExternalStore(store.subscribe, store.getSnapshot, store.getSnapshot);
  const projectedMessages = state.kind === 'ready' ? state.room.messages : [];
  const readOnlyFederatedEvents = state.kind === 'ready' ? state.room.readOnlyFederatedEvents : [];
  const projectedSignals = useMemo(
    () =>
      projectMessageSignals(
        projectedMessages.filter((message) => message.preview?.conversation === undefined),
        variant === 'direct' ? 'direct' : 'room',
      ),
    [projectedMessages, variant],
  );
  const selectedMessage =
    projectedMessages.find((message) => message.messageId === selectedMessageId) ?? null;
  const selectedSignalId = selectedMessageId === null ? null : `message:${selectedMessageId}`;
  const displayedEventId = useRef<string | null>(null);

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

  useEffect(() => {
    const latestEventId = projectedMessages[0]?.matrixEventId ?? null;
    if (
      onLatestDisplayed === undefined ||
      latestEventId === null ||
      displayedEventId.current === latestEventId
    ) {
      return;
    }
    displayedEventId.current = latestEventId;
    onLatestDisplayed(latestEventId);
  }, [onLatestDisplayed, projectedMessages]);

  return (
    <>
      <div className={`message-hub message-hub--${variant}`}>
        <ConversationPanel
          key={`chat:${roomId}`}
          writesAllowed={writesAllowed}
          publisher={messagePublisher}
          roomId={roomId}
          roomName={roomName}
          messages={projectedMessages}
          {...(participants === undefined ? {} : { participants })}
          state={state.kind === 'ready' ? 'ready' : state.kind === 'loading' ? 'loading' : 'failed'}
        />
        <ReadOnlyFederationEvents events={readOnlyFederatedEvents} />
        {state.kind === 'loading' ? null : (
          <SignalDock
            defaultExpanded={variant === 'direct'}
            onAction={handleSignalAction}
            onRetry={store.retry}
            selectedSignalId={selectedSignalId}
            signals={projectedSignals}
            state={state.kind === 'ready' ? 'ready' : 'failed'}
          />
        )}
        {writesAllowed ? (
          <MessageComposer
            key={roomId}
            publisher={messagePublisher}
            roomId={roomId}
            roomName={roomName}
          />
        ) : null}
      </div>
      <AnimatePresence>
        {selectedMessage === null ? null : (
          <ContentInspector
            catalogId={catalogId}
            contentGateway={content}
            contentVerifier={contentVerifier}
            handoffGateway={handoffs}
            key={selectedMessage.messageId}
            message={selectedMessage}
            moderationGateway={moderation}
            onClose={() => {
              onSelectedMessageChange(null);
            }}
            translationGateway={messageTranslation}
            telemetryGateway={telemetry}
          />
        )}
      </AnimatePresence>
    </>
  );
}

function ReadOnlyFederationEvents({
  events,
}: {
  readonly events: readonly ReadOnlyFederatedEvent[];
}) {
  const { t } = useTranslation();
  if (events.length === 0) {
    return null;
  }
  return (
    <details className="federation-read-only" aria-live="polite">
      <summary>
        <span>{t('messages.federationReadOnly.title')}</span>
        <span>{t('messages.federationReadOnly.count', { count: events.length })}</span>
      </summary>
      <p>{t('messages.federationReadOnly.detail')}</p>
      <ol>
        {events.map((event) => (
          <li key={event.matrixEventId}>
            <strong>{t(`messages.federationReadOnly.reason.${event.reason}`)}</strong>
            <code>{event.eventType}</code>
            <span>{event.sender}</span>
          </li>
        ))}
      </ol>
    </details>
  );
}
