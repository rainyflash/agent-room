import { Button } from '@agent-room/ui-system';
import { Bot, Search, Star } from 'lucide-react';
import {
  useDeferredValue,
  useId,
  useMemo,
  useRef,
  useState,
  type KeyboardEvent,
  type ReactNode,
} from 'react';
import { useTranslation } from 'react-i18next';

import { usePersonalWorkspace } from '@/features/personal-workspace/ui/personal-workspace-provider';
import type { RoomPeople, RoomPerson } from '@/features/private-rooms/domain/room-people';
import { formatRelativeTime } from '@/shared/i18n/formatters';

import {
  agentNameParts,
  agentRows,
  agentRowSummary,
  searchAgentRows,
  type AgentRow,
  type AgentRowState,
} from '../domain/agent-table';
import type { AgentOwner } from '../domain/agent-owners';
import type { LobbyAgent } from '../domain/lobby';
import { useAgentOwners, useAgentOwnerText } from './agent-owners';
import { AgentAvatar, HumanAvatar, type AgentAvatarState } from './avatars';
import { useNetworkAgentIds } from './network-agent-labels';
import './room-roster.css';

type RosterTab = 'agents' | 'people';
const rosterTabs: readonly RosterTab[] = ['agents', 'people'];
const pageSize = 100;

const avatarStates: Readonly<Record<AgentRowState, AgentAvatarState>> = {
  waiting: 'waiting',
  wakes: 'wakes',
  on_resume: 'idle',
  unknown: 'idle',
  reconnecting: 'idle',
  offline: 'offline',
};

export type RoomRosterProps = {
  readonly agents: readonly LobbyAgent[];
  readonly observedAtUnixMs: number;
  readonly onSelectAgent: (agentId: string) => void;
  readonly selectedAgentId: string | null;
  /** 此刻叫得醒的 Agent（开着后台回复、有电脑正替它收着）。只知道你自己的。 */
  readonly wakeableAgentIds: ReadonlySet<string>;
  readonly people: RoomPeople;
  /** 没有 Agent 时那个“接入 Agent”；这间房不能再接入时为空。 */
  readonly onInviteAgent: (() => void) | null;
  /** Agent 表格下面的东西（离线归档的规则）。 */
  readonly footer?: ReactNode;
};

/**
 * 房间页右栏（`specs/redesign/room-page.md` 的“Agent 表格”）：两个标签，“Agent”是一张表
 * （名字、谁的、状态），“人”是房间里的人。点 Agent 的一行看它的详情。
 */
export function RoomRoster({
  agents,
  observedAtUnixMs,
  onSelectAgent,
  selectedAgentId,
  wakeableAgentIds,
  people,
  onInviteAgent,
  footer,
}: RoomRosterProps) {
  const { t } = useTranslation();
  const id = useId();
  const [tab, setTab] = useState<RosterTab>('agents');
  const rows = useMemo(
    () => agentRows(agents, observedAtUnixMs, wakeableAgentIds),
    [agents, observedAtUnixMs, wakeableAgentIds],
  );
  const memberCount = rows.filter((row) => !row.archived).length;
  // 谁的 Agent：服务器按自己的记录回答（specs/room-people）。“人”那一栏要数每个人带了几个，所以整间房一起问。
  const owners = useAgentOwners(agents.map((agent) => agent.agentId));
  const counts: Readonly<Record<RosterTab, number>> = {
    agents: memberCount,
    people: people.people.length,
  };
  return (
    <section className="room-roster">
      <div aria-label={t('roomRoster.tabs')} className="room-roster__tabs" role="tablist">
        {rosterTabs.map((name, index) => (
          <button
            aria-controls={`${id}-${name}`}
            aria-selected={tab === name}
            id={`${id}-${name}-tab`}
            key={name}
            onClick={() => {
              setTab(name);
            }}
            onKeyDown={(event) => {
              const next = tabForKey(event.key, index);
              if (next === null) return;
              event.preventDefault();
              setTab(next);
              document.getElementById(`${id}-${next}-tab`)?.focus();
            }}
            role="tab"
            tabIndex={tab === name ? 0 : -1}
            type="button"
          >
            {/* 中间是真的空格：读屏读成“Agent 5”，不是“Agent5”。 */}
            {t(`roomRoster.tab.${name}`)} <span className="room-roster__count">{counts[name]}</span>
          </button>
        ))}
      </div>
      {tab === 'agents' ? (
        <div
          aria-labelledby={`${id}-agents-tab`}
          className="room-roster__panel"
          id={`${id}-agents`}
          role="tabpanel"
        >
          <AgentTable
            footer={footer}
            now={observedAtUnixMs}
            onInviteAgent={onInviteAgent}
            onSelectAgent={onSelectAgent}
            owners={owners}
            rows={rows}
            selectedAgentId={selectedAgentId}
          />
        </div>
      ) : (
        <div
          aria-labelledby={`${id}-people-tab`}
          className="room-roster__panel"
          id={`${id}-people`}
          role="tabpanel"
        >
          <PeopleTable owners={owners} people={people} />
        </div>
      )}
    </section>
  );
}

function tabForKey(key: string, index: number): RosterTab | null {
  const last = rosterTabs.length - 1;
  const target =
    key === 'ArrowRight'
      ? (index + 1) % rosterTabs.length
      : key === 'ArrowLeft'
        ? (index + last) % rosterTabs.length
        : key === 'Home'
          ? 0
          : key === 'End'
            ? last
            : null;
  return target === null ? null : (rosterTabs[target] ?? null);
}

function AgentTable({
  footer,
  now,
  onInviteAgent,
  onSelectAgent,
  owners,
  rows,
  selectedAgentId,
}: {
  readonly footer: ReactNode;
  readonly now: number;
  readonly onInviteAgent: (() => void) | null;
  readonly onSelectAgent: (agentId: string) => void;
  readonly owners: ReadonlyMap<string, AgentOwner>;
  readonly rows: readonly AgentRow[];
  readonly selectedAgentId: string | null;
}) {
  const { i18n, t } = useTranslation();
  const personal = usePersonalWorkspace();
  const ownerText = useAgentOwnerText();
  const [query, setQuery] = useState('');
  const deferredQuery = useDeferredValue(query);
  const [wantsArchived, setWantsArchived] = useState(false);
  const [page, setPage] = useState(0);
  const buttons = useRef(new Map<string, HTMLButtonElement>());
  const summary = agentRowSummary(rows);
  const archivedCount = rows.filter((row) => row.archived).length;
  const memberCount = rows.length - archivedCount;
  // 归档的都回来了（重新上线、规则改宽了）就不再停在空的归档里。
  const archivedView = wantsArchived && archivedCount > 0;
  const matching = useMemo(
    () =>
      searchAgentRows(
        rows.filter((row) => row.archived === archivedView),
        deferredQuery,
      ),
    [archivedView, deferredQuery, rows],
  );
  const pageCount = Math.max(1, Math.ceil(matching.length / pageSize));
  const currentPage = Math.min(page, pageCount - 1);
  const visible = matching.slice(currentPage * pageSize, (currentPage + 1) * pageSize);
  // 名单没问到主人时，网络 Agent 照旧认得出来（另一份查询，不分房间）。
  const networkAgents = useNetworkAgentIds(visible.map((row) => row.agent.agentId));

  const archiveToggle =
    archivedCount === 0 ? null : (
      <button
        className="room-roster__archive"
        onClick={() => {
          setWantsArchived(!archivedView);
          setPage(0);
        }}
        type="button"
      >
        {archivedView
          ? t('agentState.members', { count: memberCount })
          : t('agentState.archived', { count: archivedCount })}
      </button>
    );

  // 一个都没有（归档的不算）：一句话加“接入 Agent”。
  if (memberCount === 0 && !archivedView) {
    return (
      <div className="room-roster__scroll">
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
        {archiveToggle}
        {footer}
      </div>
    );
  }

  const stateText = (row: AgentRow): ReactNode => {
    if (row.state !== 'offline' || row.offlineSinceUnixMs === null)
      return t(`roomRoster.state.${row.state}`);
    // 离线的写离线了多久；“离线”两个字留给读屏，看得见的是整行变灰。
    return (
      <>
        <span className="sr-only">{t('roomRoster.state.offline')} · </span>
        {formatRelativeTime(row.offlineSinceUnixMs, now, i18n.resolvedLanguage)}
      </>
    );
  };
  const moveFocus = (event: KeyboardEvent, agentId: string): void => {
    const index = visible.findIndex((row) => row.agent.agentId === agentId);
    const targets: Readonly<Record<string, number>> = {
      ArrowDown: Math.min(visible.length - 1, index + 1),
      ArrowUp: Math.max(0, index - 1),
      End: visible.length - 1,
      Home: 0,
    };
    const target = visible[targets[event.key] ?? -1];
    if (index < 0 || target === undefined) return;
    event.preventDefault();
    buttons.current.get(target.agent.agentId)?.focus();
  };

  return (
    <>
      <p className="room-roster__summary">
        {archivedView ? (
          t('agentState.archiveHint')
        ) : (
          <>
            <span>{t('roomRoster.summary.waiting', { count: summary.waiting })}</span>
            {summary.wakes === 0 ? null : (
              <span>{t('roomRoster.summary.wakes', { count: summary.wakes })}</span>
            )}
            <span>{t('roomRoster.summary.offline', { count: summary.offline })}</span>
          </>
        )}
      </p>
      <label className="room-roster__search">
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
      <div className="room-roster__scroll">
        {visible.length === 0 ? (
          <p className="room-roster__none">{t('roomRoster.noMatch')}</p>
        ) : (
          <table className="agent-table agent-table--agents">
            <caption className="sr-only">{t('roomWorkspace.members')}</caption>
            <thead>
              <tr>
                <th scope="col">{t('roomRoster.column.name')}</th>
                <th scope="col">{t('roomRoster.column.owner')}</th>
                <th scope="col">{t('roomRoster.column.state')}</th>
              </tr>
            </thead>
            <tbody>
              {visible.map((row) => {
                const { agent } = row;
                const selected = agent.agentId === selectedAgentId;
                const owner =
                  ownerText.short(owners.get(agent.agentId)) ??
                  (networkAgents.has(agent.agentId) ? t('lobby.owner.networkShort') : null);
                const name = agentNameParts(agent.displayName);
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
                      <button
                        aria-pressed={selected}
                        className="agent-table__name"
                        data-agent-id={agent.agentId}
                        onClick={(event) => {
                          // 整行都能点；按钮自己点了，别让那一行再点一次。
                          event.stopPropagation();
                          onSelectAgent(agent.agentId);
                        }}
                        onKeyDown={(event) => {
                          moveFocus(event, agent.agentId);
                        }}
                        ref={(element) => {
                          if (element === null) buttons.current.delete(agent.agentId);
                          else buttons.current.set(agent.agentId, element);
                        }}
                        type="button"
                      >
                        <AgentAvatar id={agent.agentId} state={avatarStates[row.state]} />
                        <span className="agent-table__who">
                          <span className="agent-table__label">
                            <span>{name.base}</span>
                            {name.tag === '' ? null : <span>{name.tag}</span>}
                          </span>
                          {/* 窄的时候“谁的”那一列收起来，写在名字下面这一行（样式里按宽度换）。 */}
                          {owner === null ? null : (
                            <span className="agent-table__owner-line">{owner}</span>
                          )}
                        </span>
                        {personal?.snapshot.index.favorites.has(agent.agentId) === true ? (
                          <Star
                            aria-label={t('personal.favorites')}
                            className="agent-table__favorite"
                            fill="currentColor"
                          />
                        ) : null}
                      </button>
                    </th>
                    <td className="agent-table__owner">{owner}</td>
                    <td className="agent-table__state">{stateText(row)}</td>
                  </tr>
                );
              })}
            </tbody>
          </table>
        )}
        {pageCount > 1 ? (
          <nav aria-label={t('agentState.pages')} className="room-roster__pages">
            <button
              disabled={currentPage === 0}
              onClick={() => {
                setPage(currentPage - 1);
              }}
              type="button"
            >
              {t('agentState.previous')}
            </button>
            <span aria-live="polite">
              {t('agentState.page', { current: currentPage + 1, total: pageCount })}
            </span>
            <button
              disabled={currentPage + 1 >= pageCount}
              onClick={() => {
                setPage(currentPage + 1);
              }}
              type="button"
            >
              {t('agentState.next')}
            </button>
          </nav>
        ) : null}
        {archiveToggle}
        {footer}
      </div>
    </>
  );
}

function PeopleTable({
  owners,
  people,
}: {
  readonly owners: ReadonlyMap<string, AgentOwner>;
  readonly people: RoomPeople;
}) {
  const { t } = useTranslation();
  const ownerText = useAgentOwnerText();
  /** 他带了几个 Agent：照服务器说的主人数。没问到主人、认不出这个人时不写。 */
  const agentCount = (person: RoomPerson): number | null => {
    if (owners.size === 0) return null;
    const all = [...owners.values()];
    if (person.principalId !== null)
      return all.filter(
        (owner) => owner.kind === 'local' && owner.owner?.principalId === person.principalId,
      ).length;
    return person.self ? all.filter((owner) => ownerText.mine(owner)).length : null;
  };
  return (
    <div className="room-roster__scroll">
      {people.people.length === 0 ? (
        <p className="room-roster__none">{t('roomRoster.people.empty')}</p>
      ) : (
        <table className="agent-table agent-table--people">
          <caption className="sr-only">{t('roomRoster.people.caption')}</caption>
          <thead>
            <tr>
              <th scope="col">{t('roomRoster.column.nickname')}</th>
              <th scope="col">{t('roomRoster.column.role')}</th>
              <th scope="col">{t('roomRoster.column.agents')}</th>
            </tr>
          </thead>
          <tbody>
            {people.people.map((person) => (
              <tr key={person.key}>
                <th scope="row">
                  <span className="agent-table__name">
                    <HumanAvatar name={person.displayName} />
                    <span className="agent-table__who">
                      <span className="agent-table__label">
                        <span>{person.displayName}</span>
                        {person.self ? (
                          <span className="agent-table__you"> {t('roomRoster.people.you')}</span>
                        ) : null}
                      </span>
                    </span>
                  </span>
                </th>
                <td>{person.role === null ? null : t(`roomRoster.role.${person.role}`)}</td>
                <td className="agent-table__number">{agentCount(person)}</td>
              </tr>
            ))}
          </tbody>
        </table>
      )}
      {people.listed ? null : (
        <p className="room-roster__note">{t('roomRoster.people.spokenOnly')}</p>
      )}
    </div>
  );
}
