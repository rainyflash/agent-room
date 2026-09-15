import { ConversationPanel } from '@/features/conversation/ui/conversation-panel';
import type { ConversationParticipant } from '@/features/conversation/domain/conversation';
import { AnimatePresence } from 'motion/react';
import { useMemo, useState } from 'react';
import { useTranslation } from 'react-i18next';

import { useAppServices } from '@/app/app-services';
import { useRoomMessages } from './room-messages-context';
import type { MessageGateway, ReadOnlyFederatedEvent } from '@/features/messages/domain/message';
import { ContentInspector } from '@/features/messages/ui/content-inspector';
import { MessageComposer } from '@/features/messages/ui/message-composer';
import { projectMessageSignals } from '@/features/signals/adapters/message-signal-projector';
import type { SignalAction } from '@/features/signals/domain/signal';
import { SignalDock } from '@/features/signals/ui/signal-dock';

export type MessageLayerProps = {
  readonly active?: boolean;
  readonly focusedConversationMessageId?: string | null;
  readonly view?: 'conversation' | 'resources';
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
  active = true,
  focusedConversationMessageId = null,
  view = 'conversation',
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
  const { t } = useTranslation();
  const {
    content,
    contentVerifier,
    handoffs,
    messagePublisher,
    messages: messageGateway,
    messageTranslation,
    moderation,
    telemetry,
  } = useAppServices();
  const { state, store } = useRoomMessages(roomId);
  const loadOlder = messageGateway.loadOlder?.bind(messageGateway);
  const projectedMessages = state.kind === 'ready' ? state.room.messages : [];
  const latestMessage = projectedMessages[0];
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

  const handleSignalAction = (action: SignalAction): void => {
    if (action.kind === 'open_message') {
      onSelectedMessageChange(action.messageId);
    }
  };

  return (
    <>
      <div className={`message-workspace message-workspace--${variant}`} data-view={view}>
        <div className="message-workspace__conversation" hidden={view !== 'conversation'}>
          <ConversationPanel
            active={active && view === 'conversation'}
            history={state.kind === 'ready' ? state.room.history : undefined}
            {...(loadOlder ? { onLoadOlder: () => loadOlder(roomId) } : {})}
            {...(onLatestDisplayed === undefined ? {} : { onLatestDisplayed })}
            focusMessageId={
              focusedConversationMessageId ?? (view === 'conversation' ? selectedMessageId : null)
            }
            variant={variant}
            key={`chat:${roomId}`}
            writesAllowed={writesAllowed}
            publisher={messagePublisher}
            roomId={roomId}
            roomName={roomName}
            messages={projectedMessages}
            {...(participants === undefined ? {} : { participants })}
            state={
              state.kind === 'ready' ? 'ready' : state.kind === 'loading' ? 'loading' : 'failed'
            }
          />
        </div>
        <section className="message-workspace__resources" hidden={view !== 'resources'}>
          <header className="message-workspace__intro">
            <h2>
              {t(
                variant === 'direct'
                  ? 'roomWorkspace.privateResources'
                  : 'roomWorkspace.resourcesTitle',
              )}
            </h2>
            <p>{t('roomWorkspace.resourcesDetail')}</p>
          </header>
          <ReadOnlyFederationEvents events={readOnlyFederatedEvents} />
          {state.kind === 'ready' ? (
            <ResourceHistory
              key={`history:${roomId}`}
              roomId={roomId}
              gateway={messageGateway}
              canLoadMore={state.room.history?.canLoadMore === true}
              missing={selectedMessageId !== null && selectedMessage === null}
            />
          ) : null}
          {state.kind === 'loading' ? null : (
            <SignalDock
              defaultExpanded
              embedded
              onAction={handleSignalAction}
              {...(onLatestDisplayed === undefined || !active || view !== 'resources'
                ? {}
                : {
                    onFeaturedDisplayed: (signalId: string): void => {
                      if (
                        latestMessage !== undefined &&
                        signalId === `message:${latestMessage.messageId}`
                      )
                        onLatestDisplayed(latestMessage.matrixEventId);
                    },
                  })}
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
        </section>
      </div>
      <AnimatePresence>
        {selectedMessage === null || selectedMessage.preview?.conversation !== undefined ? null : (
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

function ResourceHistory({
  roomId,
  gateway,
  canLoadMore,
  missing,
}: {
  readonly roomId: string;
  readonly gateway: MessageGateway;
  readonly canLoadMore: boolean;
  readonly missing: boolean;
}) {
  const { t } = useTranslation();
  const [busy, setBusy] = useState(false);
  const [failed, setFailed] = useState(false);
  if (!canLoadMore && !missing && !failed) return null;
  return (
    <div className="conversation-history">
      {missing ? <p role="status">{t('history.positionMissing')}</p> : null}
      {failed ? <p role="alert">{t('history.failed')}</p> : null}
      {canLoadMore && gateway.loadOlder ? (
        <button
          type="button"
          disabled={busy}
          onClick={() => {
            setBusy(true);
            setFailed(false);
            void gateway.loadOlder?.(roomId).then(
              (result) => {
                setBusy(false);
                setFailed(!result.ok);
              },
              () => {
                setBusy(false);
                setFailed(true);
              },
            );
          }}
        >
          {t(busy ? 'history.loading' : 'history.older')}
        </button>
      ) : null}
    </div>
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
