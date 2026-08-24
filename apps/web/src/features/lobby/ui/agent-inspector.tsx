import { Button, StatusMark, type StatusTone } from '@agent-room/ui-system';
import { MessageSquare, ShieldBan, X } from 'lucide-react';
import { motion, useReducedMotion } from 'motion/react';
import { useTranslation } from 'react-i18next';

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
  readonly agent: LobbyAgent;
  readonly onBlock?: (agentId: string) => void;
  readonly onClose: () => void;
  readonly onMessage?: (agentId: string) => void;
};

export function AgentInspector({ agent, onBlock, onClose, onMessage }: AgentInspectorProps) {
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
          className="inspector-close"
          onClick={onClose}
          type="button"
        >
          <X aria-hidden="true" />
        </button>
      </header>
      <div className="agent-inspector__status">
        <StatusMark
          label={t(`lobby.status.${agent.status}`)}
          pulse={agent.status === 'working'}
          tone={STATUS_TONE[agent.status]}
        />
        <strong>{t(`lobby.status.${agent.status}`)}</strong>
        <span>{t('lobby.inspector.until', { time: expiry })}</span>
      </div>
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
      <section className="agent-inspector__summary">
        <h3>{t('lobby.inspector.summary')}</h3>
        <p>{agent.summary ?? t('lobby.inspector.noSummary')}</p>
      </section>
      {onMessage === undefined && onBlock === undefined ? null : (
        <div className="agent-inspector__actions">
          {onMessage === undefined ? null : (
            <Button
              icon={<MessageSquare aria-hidden="true" />}
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
              icon={<ShieldBan aria-hidden="true" />}
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
      <p className="agent-inspector__notice">{t('lobby.inspector.unverifiedNotice')}</p>
    </motion.aside>
  );
}
