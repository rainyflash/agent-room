import { Button } from '@agent-room/ui-system';
import { Bot, Search } from 'lucide-react';
import { useDeferredValue, useId, useMemo, useRef, useState, type ReactNode } from 'react';
import { useTranslation } from 'react-i18next';

import { projectTags } from '@/features/personal-workspace/domain/agent-organization';
import { usePersonalWorkspace } from '@/features/personal-workspace/ui/personal-workspace-provider';
import { formatDayAndTime } from '@/shared/i18n/formatters';

import {
  agentJoinedAtUnixMs,
  agentOrigin,
  agentOwnerFilters,
  agentRowForKey,
  agentRows,
  agentRowSummary,
  filterAgentRows,
  noAgentRowFilter,
  searchAgentRows,
  type AgentMarkFilter,
  type AgentRowFilter,
  type AgentStateFilter,
} from '../domain/agent-table';
import type { LobbyAgent } from '../domain/lobby';
import { useAgentOwners, useAgentOwnerText } from './agent-owners';
import { AgentNameButton, AgentStateText } from './agent-table-cells';
import { useNetworkAgentIds } from './network-agent-labels';
import './room-roster.css';
import './agent-table-page.css';

/** 一次列多少行；多的点“再看”接着往下列。 */
const batchSize = 100;

export type AgentTablePageProps = {
  readonly agents: readonly LobbyAgent[];
  readonly observedAtUnixMs: number;
  readonly onSelectAgent: (agentId: string) => void;
  readonly selectedAgentId: string | null;
  /** 此刻叫得醒的 Agent（开着后台回复、有电脑正替它收着）。只知道你自己的。 */
  readonly wakeableAgentIds: ReadonlySet<string>;
  /** 这台电脑上接进来的 Agent；不知道（浏览器里、还没读到）时为空。 */
  readonly localAgentIds: ReadonlySet<string> | null;
  /** 没有 Agent 时那个“接入 Agent”；这间房不能再接入时为空。 */
  readonly onInviteAgent: (() => void) | null;
  /** 表格下面的东西（离线归档的规则）。 */
  readonly footer?: ReactNode;
};

/**
 * Agent 表格展开成的整页（`specs/redesign/room-page.md` 的“Agent 表格”）：右栏那张表格多出三列
 * （怎么来的、进来的时间、你的标签），上面能按状态、谁的、你做的记号筛。点一行看详情。
 */
export function AgentTablePage({
  agents,
  footer,
  localAgentIds,
  observedAtUnixMs,
  onInviteAgent,
  onSelectAgent,
  selectedAgentId,
  wakeableAgentIds,
}: AgentTablePageProps) {
  const { i18n, t } = useTranslation();
  const headingId = useId();
  const index = usePersonalWorkspace()?.snapshot.index ?? null;
  const ownerText = useAgentOwnerText();
  const agentIds = useMemo(() => agents.map((agent) => agent.agentId), [agents]);
  // 要按“谁的”筛，所以整间房一起问（两处都是攒成一批问、问过就记着）。
  const owners = useAgentOwners(agentIds);
  const networkAgents = useNetworkAgentIds(agentIds);
  const rows = useMemo(
    () => agentRows(agents, observedAtUnixMs, wakeableAgentIds),
    [agents, observedAtUnixMs, wakeableAgentIds],
  );
  const [query, setQuery] = useState('');
  const deferredQuery = useDeferredValue(query);
  const [wanted, setWanted] = useState<AgentRowFilter>(noAgentRowFilter);
  const [shown, setShown] = useState(batchSize);
  const buttons = useRef(new Map<string, HTMLButtonElement>());

  const summary = agentRowSummary(rows);
  const archivedCount = rows.filter((row) => row.archived).length;
  const memberCount = rows.length - archivedCount;
  const tags = useMemo(() => projectTags(agents, index), [agents, index]);
  const hasFavorites = index !== null && agents.some((agent) => index.favorites.has(agent.agentId));
  // 筛着的东西没了（归档的都回来了、收藏和标签摘掉了）就回到“全部”，不停在空表上。
  const filter: AgentRowFilter = {
    state: wanted.state === 'archived' && archivedCount === 0 ? 'all' : wanted.state,
    owner: wanted.owner,
    mark:
      (wanted.mark.kind === 'favorites' && !hasFavorites) ||
      (wanted.mark.kind === 'tag' && !tags.includes(wanted.mark.tag))
        ? noAgentRowFilter.mark
        : wanted.mark,
  };
  const isNetwork = (agentId: string): boolean =>
    owners.get(agentId)?.kind === 'network' || networkAgents.has(agentId);
  const matching = searchAgentRows(
    filterAgentRows(rows, filter, {
      favorite: (agentId) => index?.favorites.has(agentId) === true,
      mine: (agentId) => ownerText.mine(owners.get(agentId)),
      network: isNetwork,
      tags: (agentId) => index?.tags.get(agentId) ?? [],
    }),
    deferredQuery,
  );
  const visible = matching.slice(0, shown);
  const change = (next: Partial<AgentRowFilter>): void => {
    setWanted({ ...filter, ...next });
    setShown(batchSize);
  };

  const stateOptions: readonly AgentStateFilter[] =
    archivedCount === 0
      ? ['all', 'waiting', 'wakes', 'offline']
      : ['all', 'waiting', 'wakes', 'offline', 'archived'];
  const markOptions: readonly AgentMarkFilter[] = [
    { kind: 'all' },
    ...(hasFavorites ? [{ kind: 'favorites' } as const] : []),
    ...tags.map((tag) => ({ kind: 'tag', tag }) as const),
  ];
  const markLabel = (mark: AgentMarkFilter): string =>
    mark.kind === 'tag' ? mark.tag : t(`roomRoster.filter.mark.${mark.kind}`);
  const markKey = (mark: AgentMarkFilter): string =>
    mark.kind === 'tag' ? `tag:${mark.tag}` : mark.kind;

  return (
    <section aria-labelledby={headingId} className="agent-page">
      <div className="agent-page__inner">
        <header className="agent-page__head">
          <div>
            {/* 从右栏展开过来时焦点落在这里（房间页放的），读屏接着读这一页。 */}
            <h2 id={headingId} tabIndex={-1}>
              {t('roomWorkspace.members')}
            </h2>
            <p className="agent-page__summary">
              <span>{t('roomRoster.summary.waiting', { count: summary.waiting })}</span>
              {summary.wakes === 0 ? null : (
                <span>{t('roomRoster.summary.wakes', { count: summary.wakes })}</span>
              )}
              <span>{t('roomRoster.summary.offline', { count: summary.offline })}</span>
            </p>
          </div>
          <label className="agent-page__search">
            <Search aria-hidden="true" />
            <span className="sr-only">{t('lobby.roster.search')}</span>
            <input
              onChange={(event) => {
                setQuery(event.currentTarget.value);
                setShown(batchSize);
              }}
              placeholder={t('lobby.roster.searchPlaceholder')}
              type="search"
              value={query}
            />
          </label>
        </header>
        {memberCount === 0 && archivedCount === 0 ? (
          <div className="room-roster__empty">
            <strong>{t('roomRoster.empty.title')}</strong>
            <p>{t('roomRoster.empty.body')}</p>
            {onInviteAgent === null ? null : (
              <Button
                icon={<Bot aria-hidden="true" />}
                onClick={onInviteAgent}
                size="compact"
                tone="ghost"
              >
                {t('studio.inviteAgent')}
              </Button>
            )}
          </div>
        ) : (
          <>
            <div className="agent-page__filters">
              <FilterGroup label={t('roomRoster.filterLabel.state')}>
                {stateOptions.map((state) => (
                  <FilterOption
                    key={state}
                    onSelect={() => {
                      change({ state });
                    }}
                    selected={filter.state === state}
                  >
                    {state === 'archived'
                      ? t('agentState.archived', { count: archivedCount })
                      : t(`roomRoster.filter.state.${state}`)}
                  </FilterOption>
                ))}
              </FilterGroup>
              <FilterGroup label={t('roomRoster.filterLabel.owner')}>
                {agentOwnerFilters.map((owner) => (
                  <FilterOption
                    key={owner}
                    onSelect={() => {
                      change({ owner });
                    }}
                    selected={filter.owner === owner}
                  >
                    {t(`roomRoster.filter.owner.${owner}`)}
                  </FilterOption>
                ))}
              </FilterGroup>
              {/* 你没收藏过、也没打过标签时，这一组没有东西可筛。 */}
              {markOptions.length === 1 ? null : (
                <FilterGroup label={t('roomRoster.filterLabel.mark')}>
                  {markOptions.map((mark) => (
                    <FilterOption
                      key={markKey(mark)}
                      onSelect={() => {
                        change({ mark });
                      }}
                      selected={markKey(filter.mark) === markKey(mark)}
                    >
                      {markLabel(mark)}
                    </FilterOption>
                  ))}
                </FilterGroup>
              )}
            </div>
            {visible.length === 0 ? (
              <p className="agent-page__none">{t('roomRoster.noMatch')}</p>
            ) : (
              <table className="agent-table agent-table--page">
                <caption className="sr-only">{t('roomWorkspace.members')}</caption>
                <thead>
                  <tr>
                    <th scope="col">{t('roomRoster.column.name')}</th>
                    <th scope="col">{t('roomRoster.column.owner')}</th>
                    <th scope="col">{t('roomRoster.column.state')}</th>
                    <th scope="col">{t('roomRoster.column.origin')}</th>
                    <th scope="col">{t('roomRoster.column.joined')}</th>
                    <th scope="col">{t('roomRoster.column.marks')}</th>
                  </tr>
                </thead>
                <tbody>
                  {visible.map((row) => {
                    const { agent } = row;
                    const selected = agent.agentId === selectedAgentId;
                    const known = owners.get(agent.agentId);
                    const network = isNetwork(agent.agentId);
                    const owner =
                      ownerText.short(known) ?? (network ? t('lobby.owner.networkShort') : null);
                    const origin = agentOrigin(agent.agentId, network, localAgentIds);
                    // 网络 Agent 经谁放行进来的（敲门的那种），服务器说了就写上。
                    const admitted = origin === 'network' ? ownerText.detail(known) : null;
                    const joinedAt = agentJoinedAtUnixMs(agent);
                    const agentTags = index?.tags.get(agent.agentId) ?? [];
                    return (
                      <tr
                        data-selected={selected ? 'true' : undefined}
                        data-state={row.state}
                        key={agent.agentId}
                        onClick={() => {
                          onSelectAgent(agent.agentId);
                        }}
                      >
                        <th scope="row">
                          <AgentNameButton
                            buttonRef={(element) => {
                              if (element === null) buttons.current.delete(agent.agentId);
                              else buttons.current.set(agent.agentId, element);
                            }}
                            // 收藏在最后一列和标签写在一起，名字后面不再画星。
                            favorite={false}
                            onKeyDown={(event) => {
                              const target = agentRowForKey(visible, agent.agentId, event.key);
                              if (target === null) return;
                              event.preventDefault();
                              buttons.current.get(target)?.focus();
                            }}
                            onSelect={() => {
                              onSelectAgent(agent.agentId);
                            }}
                            ownerLine={owner}
                            row={row}
                            selected={selected}
                          />
                        </th>
                        <td className="agent-table__owner">{owner}</td>
                        <td className="agent-table__state">
                          <AgentStateText now={observedAtUnixMs} row={row} />
                        </td>
                        <td className="agent-table__origin">
                          {t(`roomRoster.origin.${origin}`)}
                          {admitted === null ? null : ` · ${admitted}`}
                        </td>
                        <td className="agent-table__joined">
                          {joinedAt === null
                            ? null
                            : formatDayAndTime(joinedAt, observedAtUnixMs, i18n.resolvedLanguage)}
                        </td>
                        <td className="agent-table__marks">
                          {index?.favorites.has(agent.agentId) === true ? (
                            <span>{t('roomRoster.mark.favorite')}</span>
                          ) : null}
                          {agentTags.map((tag) => (
                            <span key={tag}>{tag}</span>
                          ))}
                        </td>
                      </tr>
                    );
                  })}
                </tbody>
              </table>
            )}
            {matching.length <= batchSize ? null : (
              <p className="agent-page__more">
                <span aria-live="polite">
                  {t('roomRoster.page.shown', { shown: visible.length, total: matching.length })}
                </span>
                {visible.length === matching.length ? null : (
                  <Button
                    onClick={() => {
                      setShown(shown + batchSize);
                    }}
                    size="compact"
                    tone="ghost"
                  >
                    {t('roomRoster.page.more', {
                      count: Math.min(batchSize, matching.length - visible.length),
                    })}
                  </Button>
                )}
              </p>
            )}
          </>
        )}
        {footer}
      </div>
    </section>
  );
}

function FilterGroup({
  children,
  label,
}: {
  readonly children: ReactNode;
  readonly label: string;
}) {
  return (
    <div aria-label={label} className="agent-page__filter" role="group">
      <span aria-hidden="true">{label}</span>
      {children}
    </div>
  );
}

function FilterOption({
  children,
  onSelect,
  selected,
}: {
  readonly children: ReactNode;
  readonly onSelect: () => void;
  readonly selected: boolean;
}) {
  return (
    <button aria-pressed={selected} onClick={onSelect} type="button">
      {children}
    </button>
  );
}
