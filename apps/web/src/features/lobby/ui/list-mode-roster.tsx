import { Search, Star } from 'lucide-react';
import {
  forwardRef,
  useDeferredValue,
  useId,
  useImperativeHandle,
  useMemo,
  useRef,
  useState,
} from 'react';
import { useTranslation } from 'react-i18next';

import { filterLobbyAgents } from '@/features/lobby/domain/agent-roster';
import {
  lobbyAgentStatuses,
  type LobbyAgent,
  type LobbyAgentStatus,
} from '@/features/lobby/domain/lobby';
import { AgentPortrait } from './room-illustration';
import { agentLifecycle, agentRosterGroup, agentRosterGroups } from '../domain/agent-attendance';
import { AgentStateLabel } from './agent-state-label';
import { useNetworkAgentIds, useNetworkAgentLabel } from './network-agent-labels';
import './agent-roster.css';
import { usePersonalWorkspace } from '@/features/personal-workspace/ui/personal-workspace-provider';
import {
  organizeAgents,
  projectTags,
  type AgentCollection,
} from '@/features/personal-workspace/domain/agent-organization';
import '@/features/personal-workspace/ui/personal-workspace.css';

export type ListModeRosterProps = {
  readonly variant?: 'full' | 'compact';
  readonly agents: readonly LobbyAgent[];
  readonly observedAtUnixMs: number;
  readonly onSelectAgent: (agentId: string) => void;
  readonly selectedAgentId: string | null;
  readonly selfAgentId?: string;
};

export type ListModeRosterHandle = {
  focusSelected(): boolean;
};

export const ListModeRoster = forwardRef<ListModeRosterHandle, ListModeRosterProps>(
  function ListModeRoster(
    { agents, observedAtUnixMs, onSelectAgent, selectedAgentId, selfAgentId, variant = 'full' },
    forwardedRef,
  ) {
    const { t } = useTranslation();
    const headingId = useId();
    const selectedButtonRef = useRef<HTMLButtonElement>(null);
    const buttonRefs = useRef(new Map<string, HTMLButtonElement>());
    const [query, setQuery] = useState('');
    const deferredQuery = useDeferredValue(query);
    const personal = usePersonalWorkspace();
    const [collection, setCollection] = useState<AgentCollection>('all');
    const [projectTag, setProjectTag] = useState('');
    const tags = useMemo(
      () => projectTags(agents, personal?.snapshot.index ?? null),
      [agents, personal?.snapshot.index],
    );
    const [status, setStatus] = useState<LobbyAgentStatus | 'all'>('all');
    const [archiveView, setArchiveView] = useState(false);
    const [page, setPage] = useState(0);
    const archivedCount = agents.filter(
      (agent) => agentLifecycle(agent, observedAtUnixMs).archived,
    ).length;
    const filteredAgents = useMemo(
      () =>
        organizeAgents(
          filterLobbyAgents(
            agents.filter(
              (agent) => agentLifecycle(agent, observedAtUnixMs).archived === archiveView,
            ),
            deferredQuery,
            status,
          ),
          personal?.snapshot.index ?? null,
          collection,
          projectTag,
        ).toSorted(
          (a, b) =>
            agentRosterGroups.indexOf(agentRosterGroup(a, observedAtUnixMs)) -
              agentRosterGroups.indexOf(agentRosterGroup(b, observedAtUnixMs)) ||
            (agentLifecycle(a, observedAtUnixMs).connection === 'offline'
              ? (b.lastActiveAtUnixMs ?? 0) - (a.lastActiveAtUnixMs ?? 0)
              : 0),
        ),
      [
        agents,
        deferredQuery,
        status,
        personal?.snapshot.index,
        collection,
        projectTag,
        archiveView,
        observedAtUnixMs,
      ],
    );
    const pageCount = Math.max(1, Math.ceil(filteredAgents.length / 100));
    const currentPage = Math.min(page, pageCount - 1);
    const visibleAgents = filteredAgents.slice(currentPage * 100, (currentPage + 1) * 100);
    const networkAgents = useNetworkAgentIds(visibleAgents.map((agent) => agent.agentId));
    const networkLabel = useNetworkAgentLabel();

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
        <div className="roster-views" aria-label={t('agentState.rosterView')}>
          <button
            type="button"
            aria-pressed={!archiveView}
            onClick={() => {
              setArchiveView(false);
              setPage(0);
            }}
          >
            {t('agentState.members', { count: agents.length - archivedCount })}
          </button>
          <button
            type="button"
            aria-pressed={archiveView}
            onClick={() => {
              setArchiveView(true);
              setPage(0);
            }}
          >
            {t('agentState.archived', { count: archivedCount })}
          </button>
        </div>
        {archiveView ? <p className="roster-archive-note">{t('agentState.archiveHint')}</p> : null}
        <div className="list-roster__filters">
          <label className="roster-search">
            <Search aria-hidden="true" />
            <span className="sr-only">{t('lobby.roster.search')}</span>
            <input
              onChange={(event) => {
                setQuery(event.currentTarget.value);
                setPage(0);
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
                const value = event.currentTarget.value;
                setPage(0);
                if (value === 'all') setStatus(value);
                else {
                  const found = lobbyAgentStatuses.find((item) => item === value);
                  if (found) setStatus(found);
                }
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
        {personal?.snapshot.accountId ? (
          <div className="roster-personal-filters">
            <select
              aria-label={t('personal.filter')}
              value={collection}
              onChange={(event) => {
                const value = event.currentTarget.value;
                setPage(0);
                if (value === 'all' || value === 'favorites' || value === 'recent')
                  setCollection(value);
              }}
            >
              <option value="all">{t('personal.allAgents')}</option>
              <option value="favorites">{t('personal.favorites')}</option>
              <option value="recent">{t('personal.recent')}</option>
            </select>
            <select
              aria-label={t('personal.project')}
              value={projectTag}
              onChange={(event) => {
                setPage(0);
                setProjectTag(event.currentTarget.value);
              }}
            >
              <option value="">{t('personal.allProjects')}</option>
              {tags.map((tag) => (
                <option key={tag} value={tag}>
                  {tag}
                </option>
              ))}
            </select>
          </div>
        ) : null}
        {filteredAgents.length === 0 ? (
          <p className="list-roster__empty">{t('lobby.roster.empty')}</p>
        ) : (
          <ul className="list-roster__list">
            {visibleAgents.map((agent, index) => (
              <li key={agent.agentId}>
                {index === 0 ||
                agentRosterGroup(visibleAgents[index - 1] ?? agent, observedAtUnixMs) !==
                  agentRosterGroup(agent, observedAtUnixMs) ? (
                  <h3 className="roster-group-heading">
                    {t(`agentState.group.${agentRosterGroup(agent, observedAtUnixMs)}`)}
                  </h3>
                ) : null}
                <button
                  aria-pressed={agent.agentId === selectedAgentId}
                  className="roster-agent"
                  onClick={() => {
                    onSelectAgent(agent.agentId);
                  }}
                  onKeyDown={(event) => {
                    const targetAgentId = targetAgentForKey(
                      visibleAgents,
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
                    <AgentPortrait id={agent.agentId} />
                  </span>
                  <span className="roster-agent__identity">
                    <strong>
                      {agent.displayName}
                      {personal?.snapshot.index.favorites.has(agent.agentId) ? (
                        <Star
                          className="roster-agent__favorite"
                          aria-label={t('personal.favorites')}
                          fill="currentColor"
                        />
                      ) : null}
                      {agent.agentId === selfAgentId ? <em>{t('lobby.agent.self')}</em> : null}
                      {networkAgents.has(agent.agentId) ? (
                        <em title={networkLabel.hint}>{networkLabel.label}</em>
                      ) : null}
                    </strong>
                    <AgentStateLabel agent={agent} now={observedAtUnixMs} />
                    <span className="roster-agent__work">{t(`lobby.status.${agent.status}`)}</span>
                  </span>
                  <span className="roster-agent__summary">
                    {agent.summary ?? t(`lobby.status.${agent.status}`)}
                    {(personal?.snapshot.index.tags.get(agent.agentId)?.length ?? 0) > 0 ? (
                      <span className="roster-agent__tags">
                        {personal?.snapshot.index.tags.get(agent.agentId)?.join(' · ')}
                      </span>
                    ) : null}
                  </span>
                  <span className="roster-agent__instances">
                    {t('lobby.agent.instances', { count: agent.instanceIds.length })}
                  </span>
                </button>
              </li>
            ))}
          </ul>
        )}
        {pageCount > 1 ? (
          <nav className="roster-pagination" aria-label={t('agentState.pages')}>
            <button
              type="button"
              disabled={currentPage === 0}
              onClick={() => {
                setPage(currentPage - 1);
              }}
            >
              {t('agentState.previous')}
            </button>
            <span aria-live="polite">
              {t('agentState.page', { current: currentPage + 1, total: pageCount })}
            </span>
            <button
              type="button"
              disabled={currentPage + 1 >= pageCount}
              onClick={() => {
                setPage(currentPage + 1);
              }}
            >
              {t('agentState.next')}
            </button>
          </nav>
        ) : null}
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
