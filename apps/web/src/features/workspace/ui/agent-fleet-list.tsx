import { Button } from '@agent-room/ui-system';
import { Link } from '@tanstack/react-router';
import { Bot, ChevronRight, PlugZap, RefreshCw } from 'lucide-react';
import { motion, useReducedMotion } from 'motion/react';
import { useTranslation } from 'react-i18next';

import type { FleetAgent } from '@/features/workspace/domain/agent-fleet';

export function AgentFleetList({
  agents,
  onRefresh,
  onSelectAgent,
  selectedAgentId,
}: {
  readonly agents: readonly FleetAgent[];
  readonly onRefresh: () => void;
  readonly onSelectAgent: (agentId: string) => void;
  readonly selectedAgentId: string | null;
}) {
  const { t } = useTranslation();
  const reduceMotion = useReducedMotion();
  return (
    <section className="workspace-fleet">
      <header>
        <div>
          <p className="eyebrow">{t('workspace.fleet.title')}</p>
          <h2>{t('workspace.fleet.detail')}</h2>
        </div>
        <Button
          aria-label={t('workspace.refresh')}
          icon={<RefreshCw aria-hidden="true" />}
          onClick={onRefresh}
          size="compact"
          tone="quiet"
        >
          {t('workspace.refresh')}
        </Button>
      </header>
      {agents.length === 0 ? (
        <div className="workspace-fleet__empty">
          <Bot aria-hidden="true" />
          <h3>{t('workspace.fleet.empty.title')}</h3>
          <p>{t('workspace.fleet.empty.detail')}</p>
          <Link className="ar-button ar-button--default ar-button--primary" to="/onboarding">
            <PlugZap aria-hidden="true" />
            {t('workspace.configure')}
          </Link>
        </div>
      ) : (
        <ol className="workspace-fleet__list">
          {agents.map((entry, index) => (
            <motion.li
              animate={{ opacity: 1, y: 0 }}
              initial={reduceMotion === true ? false : { opacity: 0, y: 8 }}
              key={entry.agent.agentId}
              transition={{ delay: index * 0.035, damping: 28, stiffness: 260, type: 'spring' }}
            >
              <button
                aria-pressed={selectedAgentId === entry.agent.agentId}
                onClick={() => onSelectAgent(entry.agent.agentId)}
                type="button"
              >
                <span className={`workspace-fleet__avatar is-${entry.status}`}>
                  <Bot aria-hidden="true" />
                </span>
                <span className="workspace-fleet__identity">
                  <strong>{entry.agent.displayName}</strong>
                  <small>{entry.agent.matrixUserId}</small>
                  <span>{entry.agent.description || entry.agent.slug}</span>
                </span>
                <span className={`workspace-fleet__state is-${entry.status}`}>
                  {t(`workspace.status.${entry.status}`)}
                </span>
                <span className="workspace-fleet__count">
                  {t('workspace.fleet.instances', { count: entry.instances.length })}
                </span>
                <ChevronRight aria-hidden="true" />
              </button>
            </motion.li>
          ))}
        </ol>
      )}
    </section>
  );
}
