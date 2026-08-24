import { StatusMark, type StatusTone } from '@agent-room/ui-system';
import { Search } from 'lucide-react';
import { forwardRef, useImperativeHandle, useMemo, useRef, useState } from 'react';
import { useTranslation } from 'react-i18next';

import type { LobbyAgent, LobbyAgentStatus } from '@/features/lobby/domain/lobby';

export type ListModeRosterProps = {
  readonly agents: readonly LobbyAgent[];
  readonly onSelectAgent: (agentId: string) => void;
  readonly selectedAgentId: string | null;
};

export type ListModeRosterHandle = {
  focusSelected(): void;
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
  function ListModeRoster({ agents, onSelectAgent, selectedAgentId }, forwardedRef) {
    const { t } = useTranslation();
    const selectedButtonRef = useRef<HTMLButtonElement>(null);
    const [query, setQuery] = useState('');
    const [status, setStatus] = useState<LobbyAgentStatus | 'all'>('all');
    const filteredAgents = useMemo(() => {
      const normalizedQuery = query.trim().toLocaleLowerCase();
      return agents.filter((agent) => {
        const matchesQuery =
          normalizedQuery.length === 0 ||
          agent.displayName.toLocaleLowerCase().includes(normalizedQuery) ||
          agent.matrixUserId.toLocaleLowerCase().includes(normalizedQuery);
        return matchesQuery && (status === 'all' || agent.status === status);
      });
    }, [agents, query, status]);

    useImperativeHandle(forwardedRef, () => ({
      focusSelected: () => {
        selectedButtonRef.current?.focus();
      },
    }));

    return (
      <section aria-labelledby="lobby-roster-title" className="list-roster">
        <header className="list-roster__header">
          <div>
            <p className="eyebrow">{t('lobby.roster.eyebrow')}</p>
            <h1 id="lobby-roster-title">{t('lobby.roster.title')}</h1>
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
                  ref={agent.agentId === selectedAgentId ? selectedButtonRef : undefined}
                  type="button"
                >
                  <span className={`roster-agent__signal roster-agent__signal--${agent.status}`}>
                    <StatusMark
                      label={t(`lobby.status.${agent.status}`)}
                      tone={STATUS_TONE[agent.status]}
                    />
                  </span>
                  <span className="roster-agent__identity">
                    <strong>{agent.displayName}</strong>
                    <span>{agent.matrixUserId}</span>
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
