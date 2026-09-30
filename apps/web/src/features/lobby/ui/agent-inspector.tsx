import { Button, Details } from '@agent-room/ui-system';
import { LoaderCircle, MessageSquare, ShieldBan, X } from 'lucide-react';
import { motion, useReducedMotion } from 'motion/react';
import { useTranslation } from 'react-i18next';
import type { ReactNode } from 'react';

import { AgentPortrait } from '@/features/lobby/ui/room-illustration';
import type { LobbyAgent } from '@/features/lobby/domain/lobby';
import {
  agentAttendance,
  agentLifecycle,
  agentRosterGroup,
  type AgentRosterGroup,
} from '../domain/agent-attendance';
import { AgentStateLabel } from './agent-state-label';
import { useIsNetworkAgent, useNetworkAgentLabel } from './network-agent-labels';
import './agent-roster.css';
import './agent-inspector.css';
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

/** 人物详情里只说离线多久；「离线」两个字上面的状态已经说过了。 */
function offlineDurationKey(group: AgentRosterGroup) {
  switch (group) {
    case 'offline_hour':
      return 'agentState.offlineFor.hour';
    case 'offline_today':
      return 'agentState.offlineFor.today';
    case 'offline_week':
      return 'agentState.offlineFor.week';
    case 'offline_older':
      return 'agentState.offlineFor.older';
    default:
      return 'agentState.offline';
  }
}

/**
 * 人物详情：头像、名字和状态在上；先是能做的事（私聊、屏蔽），再说它能不能回复、在做什么，
 * 然后是后台回复和收藏。Matrix 身份、在线连接数这些排查信息收在“身份与连接”里。
 */
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
  const network = useIsNetworkAgent(agent.agentId);
  const networkLabel = useNetworkAgentLabel();
  const attendance = agentAttendance(agent, observedAtUnixMs);
  const lifecycle = agentLifecycle(agent, observedAtUnixMs);
  const reception = lifecycle.connection === 'online' ? lifecycle.reception : lifecycle.connection;
  const work = agent.reportedStatus ?? agent.status;
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
        <div className="agent-inspector__portrait" aria-hidden="true">
          <AgentPortrait id={agent.agentId} />
        </div>
        <div className="agent-inspector__who">
          <h2 id="agent-inspector-title">{agent.displayName}</h2>
          <AgentStateLabel agent={agent} now={observedAtUnixMs} />
          {network ? (
            <p className="agent-inspector__origin" title={networkLabel.hint}>
              {networkLabel.label}
            </p>
          ) : null}
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
                size="compact"
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
                size="compact"
                tone="quiet"
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
        <section
          className="agent-reception"
          aria-label={t('agentDetails.reception')}
          data-state={reception}
        >
          <p>
            {t(
              hasBackgroundReception && lifecycle.reception !== 'waiting'
                ? 'agentState.hint.background'
                : `agentState.hint.${reception}`,
            )}
          </p>
        </section>
        {/* 离线又没说过在做什么时，“最后在做：离线”只是把上面的状态再说一遍。 */}
        {work === 'offline' && agent.summary === undefined ? null : (
          <section className="agent-inspector__doing" aria-labelledby="agent-inspector-doing">
            <h3 id="agent-inspector-doing">
              {t(
                lifecycle.connection === 'online' ? 'agentDetails.doing' : 'agentDetails.lastDoing',
              )}
            </h3>
            <p>
              {work === 'offline' ? null : <strong>{t(`lobby.status.${work}`)}</strong>}
              {agent.summary === undefined ? null : <span>{agent.summary}</span>}
            </p>
          </section>
        )}
        {lifecycle.connection === 'offline' ||
        lifecycle.archiveReason !== null ||
        lastActive !== null ? (
          <dl className="agent-lifecycle-facts">
            {lifecycle.connection === 'offline' ? (
              <div>
                <dt>{t('agentState.offlineTime')}</dt>
                <dd>{t(offlineDurationKey(agentRosterGroup(agent, observedAtUnixMs)))}</dd>
              </div>
            ) : null}
            {lastActive === null ? null : (
              <div>
                <dt>{t('agentDetails.lastConnection')}</dt>
                <dd>{lastActive}</dd>
              </div>
            )}
            {lifecycle.archiveReason !== null ? (
              <div>
                <dt>{t('agentState.rosterView')}</dt>
                <dd>{t(`agentState.reason.${lifecycle.archiveReason}`)}</dd>
              </div>
            ) : null}
          </dl>
        ) : null}
        {receptionControls}
        <AgentOrganizationControls key={agent.agentId} agentId={agent.agentId} />
        <Details className="agent-inspector__more" summary={t('roomGame.identityDetails')}>
          <dl className="agent-inspector__facts">
            <div>
              <dt>{t('lobby.inspector.matrixIdentity')}</dt>
              <dd>
                <code>{agent.matrixUserId}</code>
              </dd>
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
        </Details>
      </div>
    </motion.aside>
  );
}
