import { Button } from '@agent-room/ui-system';
import { LoaderCircle, MessageSquare, ShieldBan, X } from 'lucide-react';
import { motion, useReducedMotion } from 'motion/react';
import { useTranslation } from 'react-i18next';
import type { ReactNode } from 'react';

import { AgentPortrait } from '@/features/lobby/ui/room-illustration';
import type { LobbyAgent } from '@/features/lobby/domain/lobby';
import { agentAttendance, agentLifecycle, agentRosterGroup } from '../domain/agent-attendance';
import { AgentStateLabel } from './agent-state-label';
import './agent-roster.css';
import { AgentOrganizationControls } from '@/features/personal-workspace/ui/agent-organization-controls';

export type AgentInspectorProps = {
  readonly hasBackgroundReception?: boolean;
  readonly receptionControls?: ReactNode;
  readonly actionFailure?: string | null;
  readonly agent: LobbyAgent;
  readonly observedAtUnixMs?: number;
  readonly onBlock?: (agentId: string) => void;
  readonly onClose: () => void;
  readonly onMessage?: (agentId: string) => void;
  readonly pendingAction?: 'block' | 'message' | null;
};

export function AgentInspector({
  hasBackgroundReception = false,
  receptionControls,
  actionFailure = null,
  agent,
  observedAtUnixMs = Date.now(),
  onBlock,
  onClose,
  onMessage,
  pendingAction = null,
}: AgentInspectorProps) {
  const { i18n, t } = useTranslation();
  const reduceMotion = useReducedMotion();
  const attendance = agentAttendance(agent, observedAtUnixMs);
  const lifecycle = agentLifecycle(agent, observedAtUnixMs);
  const reception = lifecycle.connection === 'online' ? lifecycle.reception : lifecycle.connection;
  const lastActive =
    agent.lastActiveAtUnixMs === undefined
      ? null
      : new Intl.DateTimeFormat(i18n.resolvedLanguage, {
          month: 'short',
          day: 'numeric',
          hour: '2-digit',
          minute: '2-digit',
        }).format(agent.lastActiveAtUnixMs);
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
          <h2 id="agent-inspector-title">{agent.displayName}</h2>
        </div>
        <button
          aria-label={t('lobby.inspector.close')}
          autoFocus
          className="inspector-close ar-icon-button"
          onClick={onClose}
          type="button"
        >
          <X aria-hidden="true" />
        </button>
      </header>
      <div className="agent-inspector__body">
        <div className="agent-inspector__portrait" aria-hidden="true">
          <AgentPortrait id={agent.agentId} />
        </div>
        <div className="agent-inspector__status">
          <AgentStateLabel agent={agent} now={observedAtUnixMs} />
          {lastActive === null ? null : (
            <span>{t('studio.lastConnection', { time: lastActive })}</span>
          )}
        </div>
        <section
          className="agent-reception"
          aria-label={t('studio.reception')}
          data-state={reception}
        >
          <strong>{t(`agentState.${reception}`)}</strong>
          <p>
            {t(
              hasBackgroundReception && lifecycle.reception !== 'waiting'
                ? 'agentState.hint.background'
                : `agentState.hint.${reception}`,
            )}
          </p>
        </section>
        <dl className="agent-lifecycle-facts">
          <div>
            <dt>
              {t(lifecycle.connection === 'online' ? 'agentState.work' : 'agentState.lastWork')}
            </dt>
            <dd>{t(`lobby.status.${agent.reportedStatus ?? agent.status}`)}</dd>
          </div>
          {lifecycle.connection === 'offline' ? (
            <div>
              <dt>{t('agentState.offlineTime')}</dt>
              <dd>{t(`agentState.group.${agentRosterGroup(agent, observedAtUnixMs)}`)}</dd>
            </div>
          ) : null}
          {lifecycle.archiveReason !== null ? (
            <div>
              <dt>{t('agentState.rosterView')}</dt>
              <dd>{t(`agentState.reason.${lifecycle.archiveReason}`)}</dd>
            </div>
          ) : null}
        </dl>
        {receptionControls}
        <AgentOrganizationControls key={agent.agentId} agentId={agent.agentId} />
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
          <p className="agent-inspector__notice">{t('lobby.inspector.unverifiedNotice')}</p>
        </details>
      </div>
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
              {t(attendance === 'present' ? 'lobby.inspector.message' : 'studio.leaveMessage')}
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
    </motion.aside>
  );
}
