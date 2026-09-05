import { Button, StatusMark, type StatusTone } from '@agent-room/ui-system';
import { LoaderCircle, MessageSquare, ShieldBan, X } from 'lucide-react';
import { motion, useReducedMotion } from 'motion/react';
import { useTranslation } from 'react-i18next';

import { characterBodyArt } from '@/features/lobby/scene/character-art';
import { SceneShapes } from '@/features/lobby/scene/svg/scene-shapes';
import type { LobbyAgent, LobbyAgentStatus } from '@/features/lobby/domain/lobby';

const STATUS_TONE: Readonly<Record<LobbyAgentStatus, StatusTone>> = Object.freeze({
  blocked: 'alert',
  completed: 'active',
  idle: 'network',
  offline: 'offline',
  waiting_input: 'alert',
  working: 'active',
});

export type AgentInspectorProps = {
  readonly actionFailure?: string | null;
  readonly agent: LobbyAgent;
  readonly onBlock?: (agentId: string) => void;
  readonly onClose: () => void;
  readonly onMessage?: (agentId: string) => void;
  readonly pendingAction?: 'block' | 'message' | null;
};

export function AgentInspector({
  actionFailure = null,
  agent,
  onBlock,
  onClose,
  onMessage,
  pendingAction = null,
}: AgentInspectorProps) {
  const { i18n, t } = useTranslation();
  const reduceMotion = useReducedMotion();
  const expiry = new Intl.DateTimeFormat(i18n.resolvedLanguage, {
    hour: '2-digit',
    minute: '2-digit',
    second: '2-digit',
  }).format(agent.statusExpiresAtUnixMs);
  return (
    <motion.aside
      animate={{ opacity: 1, x: 0 }}
      aria-labelledby="agent-inspector-title"
      className="agent-inspector"
      exit={reduceMotion === true ? { opacity: 0 } : { opacity: 0, x: 20 }}
      initial={reduceMotion === true ? false : { opacity: 0, x: 28 }}
      transition={{ bounce: 0.12, damping: 28, stiffness: 240, type: 'spring' }}
    >
      <header className="agent-inspector__header">
        <div>
          <p className="eyebrow">{t('lobby.inspector.eyebrow')}</p>
          <h2 id="agent-inspector-title">{agent.displayName}</h2>
        </div>
        <button
          aria-label={t('lobby.inspector.close')}
          autoFocus
          className="inspector-close"
          onClick={onClose}
          type="button"
        >
          <X aria-hidden="true" />
        </button>
      </header>
      <div className="agent-inspector__portrait" aria-hidden="true">
        <svg viewBox="-45 -78 90 90">
          <ellipse cx="0" cy="2" rx="25" ry="8" fill="#c6d4b9" />
          <rect x="-11" y="-17" width="9" height="20" rx="3" fill="#47564e" />
          <rect x="2" y="-17" width="9" height="20" rx="3" fill="#47564e" />
          <SceneShapes shapes={characterBodyArt(agent.agentId)} />
        </svg>
      </div>
      <div className="agent-inspector__status">
        <StatusMark label={t(`lobby.status.${agent.status}`)} tone={STATUS_TONE[agent.status]} />
        <strong>{t(`lobby.status.${agent.status}`)}</strong>
        <span>{t('lobby.inspector.until', { time: expiry })}</span>
      </div>
      <section className="agent-inspector__summary">
        <h3>{t('lobby.inspector.summary')}</h3>
        <p>{agent.summary ?? t('lobby.inspector.noSummary')}</p>
      </section>
      <details className="agent-inspector__identity">
        <summary>{t('roomGame.identityDetails')}</summary>
        <dl className="agent-inspector__facts">
          <div>
            <dt>{t('lobby.inspector.matrixIdentity')}</dt>
            <dd>{agent.matrixUserId}</dd>
          </div>
          <div>
            <dt>{t('lobby.inspector.trust')}</dt>
            <dd>{t(`lobby.trust.${agent.trust}`)}</dd>
          </div>
          <div>
            <dt>{t('lobby.inspector.visibility')}</dt>
            <dd>{t(`lobby.visibility.${agent.visibility}`)}</dd>
          </div>
          <div>
            <dt>{t('lobby.inspector.instances')}</dt>
            <dd>{agent.instanceIds.length}</dd>
          </div>
        </dl>
      </details>
      {onMessage === undefined && onBlock === undefined ? null : (
        <div className="agent-inspector__actions">
          {onMessage === undefined ? null : (
            <Button
              disabled={pendingAction !== null}
              icon={
                pendingAction === 'message' ? (
                  <LoaderCircle aria-hidden="true" />
                ) : (
                  <MessageSquare aria-hidden="true" />
                )
              }
              onClick={() => {
                onMessage(agent.agentId);
              }}
              tone="primary"
            >
              {t('lobby.inspector.message')}
            </Button>
          )}
          {onBlock === undefined ? null : (
            <Button
              disabled={pendingAction !== null}
              icon={
                pendingAction === 'block' ? (
                  <LoaderCircle aria-hidden="true" />
                ) : (
                  <ShieldBan aria-hidden="true" />
                )
              }
              onClick={() => {
                onBlock(agent.agentId);
              }}
              tone="alert"
            >
              {t('lobby.inspector.block')}
            </Button>
          )}
        </div>
      )}
      {actionFailure === null ? null : (
        <p className="agent-inspector__failure" role="alert">
          {t('directSessions.failure', { code: actionFailure })}
        </p>
      )}
      <p className="agent-inspector__notice">{t('lobby.inspector.unverifiedNotice')}</p>
    </motion.aside>
  );
}
