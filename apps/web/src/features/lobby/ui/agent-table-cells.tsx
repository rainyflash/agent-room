import { Star } from 'lucide-react';
import type { KeyboardEvent } from 'react';
import { useTranslation } from 'react-i18next';

import { formatRelativeTime } from '@/shared/i18n/formatters';

import { agentNameParts, type AgentRow, type AgentRowState } from '../domain/agent-table';
import { AgentAvatar, type AgentAvatarState } from './avatars';

/** 右栏的表格和整页的表格共用的两格：名字那一格、状态那一格。样式在 `room-roster.css`。 */

const avatarStates: Readonly<Record<AgentRowState, AgentAvatarState>> = {
  waiting: 'waiting',
  wakes: 'wakes',
  on_resume: 'idle',
  unknown: 'idle',
  reconnecting: 'idle',
  offline: 'offline',
};

/**
 * 名字那一格里的按钮：头像、名字（太长时截前面、留着编号）、收藏的星。点它看这个 Agent 的详情。
 * `ownerLine` 是窄的时候写在名字下面的“谁的”，样式里按宽度决定显不显示。
 */
export function AgentNameButton({
  buttonRef,
  favorite,
  onKeyDown,
  onSelect,
  ownerLine,
  row,
  selected,
}: {
  readonly buttonRef: (element: HTMLButtonElement | null) => void;
  readonly favorite: boolean;
  readonly onKeyDown: (event: KeyboardEvent) => void;
  readonly onSelect: () => void;
  readonly ownerLine: string | null;
  readonly row: AgentRow;
  readonly selected: boolean;
}) {
  const { t } = useTranslation();
  const name = agentNameParts(row.agent.displayName);
  return (
    <button
      aria-pressed={selected}
      className="agent-table__name"
      data-agent-id={row.agent.agentId}
      onClick={(event) => {
        // 整行都能点；按钮自己点了，别让那一行再点一次。
        event.stopPropagation();
        onSelect();
      }}
      onKeyDown={onKeyDown}
      ref={buttonRef}
      type="button"
    >
      <AgentAvatar id={row.agent.agentId} state={avatarStates[row.state]} />
      <span className="agent-table__who">
        <span className="agent-table__label">
          <span>{name.base}</span>
          {name.tag === '' ? null : <span>{name.tag}</span>}
        </span>
        {ownerLine === null ? null : <span className="agent-table__owner-line">{ownerLine}</span>}
      </span>
      {favorite ? (
        <Star
          aria-label={t('personal.favorites')}
          className="agent-table__favorite"
          fill="currentColor"
        />
      ) : null}
    </button>
  );
}

/** 状态那一格里的字。离线的写离线了多久；“离线”两个字留给读屏，看得见的是整行淡一档。 */
export function AgentStateText({ now, row }: { readonly now: number; readonly row: AgentRow }) {
  const { i18n, t } = useTranslation();
  if (row.state !== 'offline' || row.offlineSinceUnixMs === null)
    return t(`roomRoster.state.${row.state}`);
  return (
    <>
      <span className="sr-only">{t('roomRoster.state.offline')} · </span>
      {formatRelativeTime(row.offlineSinceUnixMs, now, i18n.resolvedLanguage)}
    </>
  );
}
