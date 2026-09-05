import { initials } from '@/shared/ui/display-name';

import { Button, StatusMark } from '@agent-room/ui-system';
import {
  EyeOff,
  LoaderCircle,
  MessageCircle,
  RefreshCw,
  ShieldBan,
  ShieldCheck,
  X,
} from 'lucide-react';
import { AnimatePresence, motion, useReducedMotion } from 'motion/react';
import { useTranslation } from 'react-i18next';

import type { DirectSessionController } from '@/features/direct-sessions/ui/use-direct-session-controller';
import type { DirectSession } from '@/features/direct-sessions/domain/direct-session';
import { MessageLayer } from '@/features/messages/ui/message-layer';

export type DirectConversationDockProps = {
  readonly activeCatalogId: string | null;
  readonly controller: DirectSessionController;
  readonly onActiveSessionChange: (catalogId: string | null) => void;
  readonly onSelectedMessageChange: (messageId: string | null) => void;
  readonly selectedMessageId: string | null;
};

export function DirectConversationDock({
  activeCatalogId,
  controller,
  onActiveSessionChange,
  onSelectedMessageChange,
  selectedMessageId,
}: DirectConversationDockProps) {
  const { t } = useTranslation();
  const reduceMotion = useReducedMotion();
  const activeSession =
    activeCatalogId === null
      ? null
      : (controller.sessions.find((session) => session.catalogId === activeCatalogId) ?? null);
  const visible = controller.sessions.length > 0 || activeCatalogId !== null;

  if (!visible) {
    return null;
  }

  return (
    <section
      aria-label={t('directSessions.dock.label')}
      className={`direct-session-dock${activeCatalogId === null ? '' : ' direct-session-dock--open'}`}
    >
      <SessionRail
        activeCatalogId={activeCatalogId}
        loading={controller.loading}
        onActivate={onActiveSessionChange}
        sessions={controller.sessions}
      />
      <AnimatePresence mode="wait">
        {activeCatalogId === null ? null : activeSession === null ? (
          <motion.aside
            animate={{ opacity: 1, y: 0 }}
            className="direct-conversation direct-conversation--boundary"
            exit={{ opacity: 0, y: 8 }}
            initial={reduceMotion === true ? false : { opacity: 0, y: 14 }}
            key="missing-direct-session"
            transition={{ damping: 28, stiffness: 300, type: 'spring' }}
          >
            <button
              aria-label={t('directSessions.action.close')}
              className="direct-conversation__close"
              onClick={() => {
                onActiveSessionChange(null);
              }}
              type="button"
            >
              <X aria-hidden="true" />
            </button>
            {controller.loading ? (
              <LoaderCircle aria-hidden="true" />
            ) : (
              <EyeOff aria-hidden="true" />
            )}
            <h2>
              {t(
                controller.loading
                  ? 'directSessions.loading.title'
                  : 'directSessions.missing.title',
              )}
            </h2>
            <p>
              {t(
                controller.loading
                  ? 'directSessions.loading.detail'
                  : 'directSessions.missing.detail',
              )}
            </p>
            <Button
              disabled={controller.loading}
              icon={<RefreshCw aria-hidden="true" />}
              onClick={() => {
                void controller.retry();
              }}
              tone="quiet"
            >
              {t('directSessions.action.retry')}
            </Button>
          </motion.aside>
        ) : (
          <Conversation
            controller={controller}
            key={activeSession.catalogId}
            onClose={() => {
              onActiveSessionChange(null);
            }}
            onSelectedMessageChange={onSelectedMessageChange}
            reduceMotion={reduceMotion === true}
            selectedMessageId={selectedMessageId}
            session={activeSession}
          />
        )}
      </AnimatePresence>
    </section>
  );
}

function SessionRail({
  activeCatalogId,
  loading,
  onActivate,
  sessions,
}: {
  readonly activeCatalogId: string | null;
  readonly loading: boolean;
  readonly onActivate: (catalogId: string) => void;
  readonly sessions: readonly DirectSession[];
}) {
  const { t } = useTranslation();
  return (
    <nav aria-label={t('directSessions.rail.label')} className="direct-session-rail">
      <div className="direct-session-rail__brand" title={t('directSessions.rail.title')}>
        <MessageCircle aria-hidden="true" />
      </div>
      <div className="direct-session-rail__sessions">
        {sessions.map((session) => (
          <button
            aria-label={t('directSessions.rail.open', { name: session.target.displayName })}
            aria-pressed={activeCatalogId === session.catalogId}
            className="direct-session-rail__session"
            data-lifecycle={session.lifecycle}
            key={session.catalogId}
            onClick={() => {
              onActivate(session.catalogId);
            }}
            type="button"
          >
            <span aria-hidden="true">{initials(session.target.displayName)}</span>
            <StatusMark
              label={t(`directSessions.lifecycle.${session.lifecycle}`)}
              tone={session.lifecycle === 'active' ? 'network' : 'offline'}
            />
          </button>
        ))}
        {loading && sessions.length === 0 ? (
          <span className="direct-session-rail__loading">
            <LoaderCircle aria-hidden="true" />
          </span>
        ) : null}
      </div>
    </nav>
  );
}

function Conversation({
  controller,
  onClose,
  onSelectedMessageChange,
  reduceMotion,
  selectedMessageId,
  session,
}: {
  readonly controller: DirectSessionController;
  readonly onClose: () => void;
  readonly onSelectedMessageChange: (messageId: string | null) => void;
  readonly reduceMotion: boolean;
  readonly selectedMessageId: string | null;
  readonly session: DirectSession;
}) {
  const { t } = useTranslation();
  const principalBlocked = session.contactPolicy.principalBlocksAgent;
  const remoteBlocked = session.contactPolicy.agentBlocksPrincipal;
  const matrixRoomId = session.matrixRoomId;
  return (
    <motion.aside
      animate={{ opacity: 1, scale: 1, y: 0 }}
      aria-labelledby="direct-conversation-title"
      className="direct-conversation"
      exit={{ opacity: 0, scale: reduceMotion ? 1 : 0.992, y: reduceMotion ? 0 : 10 }}
      initial={reduceMotion ? false : { opacity: 0, scale: 0.992, y: 14 }}
      transition={{ damping: 30, stiffness: 320, type: 'spring' }}
    >
      <header className="direct-conversation__header">
        <div aria-hidden="true" className="direct-conversation__avatar">
          {initials(session.target.displayName)}
        </div>
        <div className="direct-conversation__identity">
          <p className="eyebrow">{t('directSessions.conversation.eyebrow')}</p>
          <h2 id="direct-conversation-title">{session.target.displayName}</h2>
          <span>{session.target.matrixUserId}</span>
        </div>
        <div className="direct-conversation__policy">
          <span className="direct-conversation__presence">
            <StatusMark
              label={t(
                session.contactPolicy.presenceDisclosure === 'hidden'
                  ? 'directSessions.presence.hidden'
                  : 'directSessions.presence.coarse',
              )}
              tone={session.contactPolicy.deliveryAllowed ? 'network' : 'offline'}
            />
            <span>
              {t(
                session.contactPolicy.presenceDisclosure === 'hidden'
                  ? 'directSessions.presence.hidden'
                  : 'directSessions.presence.coarse',
              )}
            </span>
          </span>
          <span className="direct-conversation__policy-detail">
            {t(
              remoteBlocked
                ? 'directSessions.policy.remoteBlocked'
                : principalBlocked
                  ? 'directSessions.policy.blocked'
                  : 'directSessions.policy.deliverable',
            )}
          </span>
        </div>
        <div className="direct-conversation__actions">
          <Button
            aria-label={t(
              principalBlocked ? 'directSessions.action.unblock' : 'directSessions.action.block',
            )}
            disabled={controller.blocking}
            icon={
              controller.blocking ? (
                <LoaderCircle aria-hidden="true" />
              ) : principalBlocked ? (
                <ShieldCheck aria-hidden="true" />
              ) : (
                <ShieldBan aria-hidden="true" />
              )
            }
            onClick={() => {
              void controller.setBlocked(session.target, !principalBlocked);
            }}
            size="compact"
            tone={principalBlocked ? 'quiet' : 'ghost'}
          >
            {t(principalBlocked ? 'directSessions.action.unblock' : 'directSessions.action.block')}
          </Button>
          <button
            aria-label={t('directSessions.action.close')}
            className="direct-conversation__close"
            onClick={onClose}
            type="button"
          >
            <X aria-hidden="true" />
          </button>
        </div>
      </header>
      {controller.failure === null ? null : (
        <div className="direct-conversation__failure" role="alert">
          <span>{t('directSessions.failure', { code: controller.failure.code })}</span>
          {controller.failure.retryable ? (
            <button
              onClick={() => {
                void controller.retry();
              }}
              type="button"
            >
              {t('directSessions.action.retry')}
            </button>
          ) : null}
        </div>
      )}
      {session.lifecycle === 'active' && matrixRoomId !== null ? (
        <div className="direct-conversation__messages">
          <div className="direct-conversation__principle">
            <ShieldCheck aria-hidden="true" />
            <strong>{t('directSessions.conversation.previewTitle')}</strong>
            <span>{t('directSessions.conversation.previewDetail')}</span>
          </div>
          <MessageLayer
            writesAllowed={session.contactPolicy.deliveryAllowed}
            participants={[session.target]}
            catalogId={session.catalogId}
            onLatestDisplayed={(matrixEventId) => {
              void controller.markDisplayed(matrixRoomId, matrixEventId);
            }}
            onSelectedMessageChange={onSelectedMessageChange}
            roomId={matrixRoomId}
            roomName={session.target.displayName}
            selectedMessageId={selectedMessageId}
            variant="direct"
          />
        </div>
      ) : (
        <div className="direct-conversation__provisioning" role="status">
          {session.lifecycle === 'provisioning' ? (
            <LoaderCircle aria-hidden="true" />
          ) : (
            <EyeOff aria-hidden="true" />
          )}
          <h3>{t(`directSessions.lifecycle.${session.lifecycle}`)}</h3>
          <p>{t(`directSessions.lifecycle.${session.lifecycle}Detail`)}</p>
        </div>
      )}
    </motion.aside>
  );
}
