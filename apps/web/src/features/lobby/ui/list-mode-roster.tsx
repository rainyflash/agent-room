import { StatusMark, type StatusTone } from '@agent-room/ui-system';
import { Search } from 'lucide-react';
import { forwardRef, useId, useImperativeHandle, useMemo, useRef, useState } from 'react';
import { useTranslation } from 'react-i18next';

import { filterLobbyAgents } from '@/features/lobby/domain/agent-roster';
import type { LobbyAgent, LobbyAgentStatus } from '@/features/lobby/domain/lobby';

export type ListModeRosterProps = {
  readonly variant?: 'full' | 'compact';
  readonly agents: readonly LobbyAgent[];
  readonly onSelectAgent: (agentId: string) => void;
  readonly selectedAgentId: string | null;
  readonly selfAgentId?: string;
};

export type ListModeRosterHandle = {
  focusSelected(): boolean;
};

const STATUS_TONE: Readonly<Record<LobbyAgentStatus, StatusTone>> = Object.freeze({
  blocked: 'alert',
  completed: 'active',
  idle: 'network',
  offline: 'offline',
  waiting_input: 'alert',
  working: 'active',
});

export const ListModeRoster = forwardRef<ListModeRosterHandle, ListModeRosterProps>(
  function ListModeRoster(
    { agents, onSelectAgent, selectedAgentId, selfAgentId, variant = 'full' },
    forwardedRef,
  ) {
    const { t } = useTranslation();
    const headingId = useId();
    const selectedButtonRef = useRef<HTMLButtonElement>(null);
    const buttonRefs = useRef(new Map<string, HTMLButtonElement>());
    const [query, setQuery] = useState('');
    const [status, setStatus] = useState<LobbyAgentStatus | 'all'>('all');
    const filteredAgents = useMemo(
      () => filterLobbyAgents(agents, query, status),
      [agents, query, status],
    );

    useImperativeHandle(forwardedRef, () => ({
      focusSelected: () => {
        const selected = selectedButtonRef.current;
        selected?.focus();
        return selected !== null && document.activeElement === selected;
      },
    }));

    return (
      <section aria-labelledby={headingId} className={`list-roster list-roster--${variant}`}>
        <header className="list-roster__header">
          <div>
            <p className="eyebrow">{t('lobby.roster.eyebrow')}</p>
            <h2 id={headingId}>
              {t(variant === 'compact' ? 'roomWorkspace.members' : 'lobby.roster.title')}
            </h2>
          </div>
          <span>{t('lobby.roster.count', { count: filteredAgents.length })}</span>
        </header>
        <div className="list-roster__filters">
          <label className="roster-search">
            <Search aria-hidden="true" />
            <span className="sr-only">{t('lobby.roster.search')}</span>
            <input
              onChange={(event) => {
                setQuery(event.currentTarget.value);
              }}
              placeholder={t('lobby.roster.searchPlaceholder')}
              type="search"
              value={query}
            />
          </label>
          <label>
            <span className="sr-only">{t('lobby.roster.filter')}</span>
            <select
              aria-label={t('lobby.roster.filter')}
              onChange={(event) => {
                setStatus(event.currentTarget.value as LobbyAgentStatus | 'all');
              }}
              value={status}
            >
              <option value="all">{t('lobby.status.all')}</option>
              {(
                ['working', 'waiting_input', 'blocked', 'idle', 'completed', 'offline'] as const
              ).map((agentStatus) => (
                <option key={agentStatus} value={agentStatus}>
                  {t(`lobby.status.${agentStatus}`)}
                </option>
              ))}
            </select>
          </label>
        </div>
        {filteredAgents.length === 0 ? (
          <p className="list-roster__empty">{t('lobby.roster.empty')}</p>
        ) : (
          <ul className="list-roster__list">
            {filteredAgents.map((agent) => (
              <li key={agent.agentId}>
                <button
                  aria-pressed={agent.agentId === selectedAgentId}
                  className="roster-agent"
                  onClick={() => {
                    onSelectAgent(agent.agentId);
                  }}
                  onKeyDown={(event) => {
                    const targetAgentId = targetAgentForKey(
                      filteredAgents,
                      agent.agentId,
                      event.key,
                    );
                    if (targetAgentId === null) {
                      return;
                    }
                    event.preventDefault();
                    buttonRefs.current.get(targetAgentId)?.focus();
                  }}
                  ref={(element) => {
                    if (element === null) {
                      buttonRefs.current.delete(agent.agentId);
                    } else {
                      buttonRefs.current.set(agent.agentId, element);
                    }
                    if (agent.agentId === selectedAgentId) {
                      selectedButtonRef.current = element;
                    }
                  }}
                  type="button"
                >
                  <span className={`roster-agent__signal roster-agent__signal--${agent.status}`}>
                    <StatusMark
                      label={t(`lobby.status.${agent.status}`)}
                      tone={STATUS_TONE[agent.status]}
                    />
                  </span>
                  <span className="roster-agent__identity">
                    <strong>
                      {agent.displayName}
                      {agent.agentId === selfAgentId ? <em>{t('lobby.agent.self')}</em> : null}
                    </strong>
                    <span>
                      {variant === 'compact'
                        ? t(`lobby.status.${agent.status}`)
                        : agent.matrixUserId}
                    </span>
                  </span>
                  <span className="roster-agent__summary">
                    {agent.summary ?? t(`lobby.status.${agent.status}`)}
                  </span>
                  <span className="roster-agent__instances">
                    {t('lobby.agent.instances', { count: agent.instanceIds.length })}
                  </span>
                </button>
              </li>
            ))}
          </ul>
        )}
      </section>
    );
  },
);

function targetAgentForKey(
  agents: readonly LobbyAgent[],
  currentAgentId: string,
  key: string,
): string | null {
  const currentIndex = agents.findIndex((agent) => agent.agentId === currentAgentId);
  if (currentIndex < 0 || agents.length === 0) {
    return null;
  }
  const targetIndexByKey: Readonly<Record<string, number>> = {
    ArrowDown: Math.min(agents.length - 1, currentIndex + 1),
    ArrowUp: Math.max(0, currentIndex - 1),
    End: agents.length - 1,
    Home: 0,
  };
  const targetIndex = targetIndexByKey[key];
  return targetIndex === undefined ? null : (agents[targetIndex]?.agentId ?? null);
}
