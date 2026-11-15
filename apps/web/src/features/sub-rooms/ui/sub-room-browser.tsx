import { Button, RoomGlyph } from '@agent-room/ui-system';
import type { TFunction } from 'i18next';
import { Check, LoaderCircle, Plus, RotateCcw, Search } from 'lucide-react';
import { useDeferredValue, useEffect, useId, useRef, useState, type ReactNode } from 'react';
import { useTranslation } from 'react-i18next';

import { useAppServices } from '@/app/app-services';
import { useCatalogUnread } from '@/features/inbox/ui/room-unread-mark';
import { usePrivateRoomPeople } from '@/features/private-rooms/data/private-room-queries';
import { useSubRooms } from '@/features/private-rooms/data/sub-room-queries';
import { JOIN_REQUEST_MESSAGE_MAX_LENGTH } from '@/features/private-rooms/domain/public-room-join';
import type { SubRoom, SubRoomList } from '@/features/private-rooms/domain/sub-rooms';
import {
  searchSubRooms,
  subRoomBrowseGroups,
  subRoomBrowseTabs,
  subRoomCategoryName,
  subRoomRowAction,
  subRoomsOnTab,
  type SubRoomBrowseTab,
} from '@/features/sub-rooms/domain/sub-room-browse';
import { formatDayAndTime } from '@/shared/i18n/formatters';

import { SubRoomLink } from './sub-room-link';
import {
  useSubRoomActions,
  type SubRoomActionKind,
  type SubRoomActions,
} from './use-sub-room-actions';
import './sub-room-browser.css';

export type SubRoomBrowserProps = {
  /** 这间房下面没归档的子房间（房间页问过的那一份）。 */
  readonly list: SubRoomList;
  readonly principalId: string | null;
  /** 右上角“新建子房间”；你建不了时为空。 */
  readonly onCreate: (() => void) | null;
  /** 点一间你还没进的名字：在这一页的位置上展开它的卡片（谁在管、门是什么样）。 */
  readonly onOpenDoor: (subRoomId: string) => void;
};

/**
 * “浏览子房间”一页（`specs/redesign/sub-rooms.md`“网页”）：这间房下面你看得到的子房间，按分类排着，
 * 能搜名字和用途；三个标签：全部、我在的、已归档。没进的在这里加入或者申请，归档的在这里打开、恢复。
 */
export function SubRoomBrowser({ list, onCreate, onOpenDoor, principalId }: SubRoomBrowserProps) {
  const { t } = useTranslation();
  const { subRooms } = useAppServices();
  const headingId = useId();
  const parentId = list.parent.catalogId;
  const [tab, setTab] = useState<SubRoomBrowseTab>('all');
  const [query, setQuery] = useState('');
  const deferredQuery = useDeferredValue(query);
  const archivedQuery = useSubRooms(subRooms, parentId, { archived: true });
  const archived = archivedQuery.data?.ok === true ? archivedQuery.data.value : null;
  const actions = useSubRoomActions(parentId);
  const counts: Record<SubRoomBrowseTab, number | null> = {
    all: list.subRooms.length,
    mine: list.subRooms.filter((room) => room.joined).length,
    archived: archived?.subRooms.length ?? null,
  };
  return (
    <section aria-labelledby={headingId} className="sub-room-browser">
      <div className="sub-room-browser__inner">
        <header className="sub-room-browser__head">
          {/* 从左栏打开时焦点落在这里（房间页放的），读屏接着读这一页。 */}
          <h2 id={headingId} tabIndex={-1}>
            {t('subRooms.browse.title')}
          </h2>
          <div className="sub-room-browser__tools">
            <label className="sub-room-browser__search">
              <Search aria-hidden="true" />
              <span className="sr-only">{t('subRooms.browse.search')}</span>
              <input
                onChange={(event) => {
                  setQuery(event.currentTarget.value);
                }}
                placeholder={t('subRooms.browse.searchPlaceholder')}
                type="search"
                value={query}
              />
            </label>
            {onCreate === null ? null : (
              <Button icon={<Plus aria-hidden="true" />} onClick={onCreate} size="compact">
                {t('subRooms.create.open')}
              </Button>
            )}
          </div>
        </header>
        <div aria-label={t('subRooms.browse.tabs')} className="sub-room-browser__tabs" role="group">
          {subRoomBrowseTabs.map((value) => (
            <button
              aria-pressed={tab === value}
              key={value}
              onClick={() => {
                setTab(value);
              }}
              type="button"
            >
              {t(`subRooms.browse.tab.${value}`)}
              {counts[value] === null ? null : (
                <span className="sub-room-browser__tab-count">{counts[value]}</span>
              )}
            </button>
          ))}
        </div>
        {tab === 'archived' ? (
          <ArchivedRooms
            actions={actions}
            archived={archived}
            failed={archivedQuery.data?.ok === false || archivedQuery.isError}
            parentId={parentId}
            principalId={principalId}
            query={deferredQuery}
          />
        ) : (
          <ActiveRooms
            actions={actions}
            list={list}
            onOpenDoor={onOpenDoor}
            query={deferredQuery}
            tab={tab}
          />
        )}
      </div>
    </section>
  );
}

function ActiveRooms({
  actions,
  list,
  onOpenDoor,
  query,
  tab,
}: {
  readonly actions: SubRoomActions;
  readonly list: SubRoomList;
  readonly onOpenDoor: (subRoomId: string) => void;
  readonly query: string;
  readonly tab: Exclude<SubRoomBrowseTab, 'archived'>;
}) {
  const { t } = useTranslation();
  // 一次只开一张申请：开了另一行的，这一张收起来。
  const [requesting, setRequesting] = useState<string | null>(null);
  const rooms = searchSubRooms(subRoomsOnTab(list.subRooms, tab), query);
  if (rooms.length === 0)
    return (
      <p className="sub-room-browser__empty">
        {query.trim() === ''
          ? t(`subRooms.browse.empty.${tab}`)
          : t('subRooms.browse.empty.search', { query: query.trim() })}
      </p>
    );
  const titled = list.categories.length > 0;
  return (
    <div className="sub-room-browser__groups">
      {subRoomBrowseGroups(list.categories, rooms).map((group) => (
        <section className="sub-room-browser__group" key={group.category?.categoryId ?? 'loose'}>
          {titled ? (
            <h3>
              {group.category?.name ?? t('subRooms.browse.uncategorized')}
              <span className="sub-room-browser__group-count">{group.rooms.length}</span>
            </h3>
          ) : null}
          <ul className="sub-room-browser__rows">
            {group.rooms.map((room) => (
              <li key={room.catalogId}>
                <ActiveRoom
                  actions={actions}
                  onCancelRequest={() => {
                    setRequesting(null);
                  }}
                  onOpenDoor={onOpenDoor}
                  onRequest={() => {
                    setRequesting(room.catalogId);
                  }}
                  requesting={requesting === room.catalogId}
                  room={room}
                />
              </li>
            ))}
          </ul>
        </section>
      ))}
    </div>
  );
}

function ActiveRoom({
  actions,
  onCancelRequest,
  onOpenDoor,
  onRequest,
  requesting,
  room,
}: {
  readonly actions: SubRoomActions;
  readonly onCancelRequest: () => void;
  readonly onOpenDoor: (subRoomId: string) => void;
  readonly onRequest: () => void;
  readonly requesting: boolean;
  readonly room: SubRoom;
}) {
  const { t } = useTranslation();
  const found = useCatalogUnread(room.catalogId);
  const action = subRoomRowAction(room);
  const busy = actions.busy?.catalogId === room.catalogId ? actions.busy.kind : null;
  const meta =
    room.memberCount !== undefined
      ? [
          t('subRooms.browse.people', { count: room.memberCount }),
          t('subRooms.browse.agents', { count: room.agentCount ?? 0 }),
        ].join(' · ')
      : room.access === 'open'
        ? null
        : t(`subRooms.browse.door.${room.access}`);
  return (
    <>
      <div className="sub-room-browser__row" data-joined={room.joined ? 'true' : undefined}>
        <Glyph room={room} />
        {room.joined ? (
          <SubRoomLink
            catalogId={room.catalogId}
            className="sub-room-browser__name"
            matrixRoomId={found?.roomId}
          >
            {room.name}
          </SubRoomLink>
        ) : (
          <button
            className="sub-room-browser__name"
            onClick={() => {
              onOpenDoor(room.catalogId);
            }}
            type="button"
          >
            {room.name}
          </button>
        )}
        <span className="sub-room-browser__purpose">{room.description}</span>
        <span className="sub-room-browser__meta">{meta}</span>
        <span className="sub-room-browser__action">
          <ActiveAction
            action={action}
            actions={actions}
            busy={busy}
            onRequest={onRequest}
            requesting={requesting}
            room={room}
          />
        </span>
      </div>
      {requesting && action === 'request' ? (
        <RequestForm
          busy={busy === 'request'}
          onCancel={onCancelRequest}
          onSend={(message) => {
            actions.request(room, message);
          }}
          room={room}
        />
      ) : null}
      <FailureNote actions={actions} room={room} />
    </>
  );
}

function ActiveAction({
  action,
  actions,
  busy,
  onRequest,
  requesting,
  room,
}: {
  readonly action: ReturnType<typeof subRoomRowAction>;
  readonly actions: SubRoomActions;
  readonly busy: SubRoomActionKind | null;
  readonly onRequest: () => void;
  readonly requesting: boolean;
  readonly room: SubRoom;
}) {
  const { t } = useTranslation();
  const name = room.name;
  switch (action) {
    case 'joined':
      return (
        <span className="sub-room-browser__state">
          <Check aria-hidden="true" />
          {t('subRooms.browse.action.joined')}
        </span>
      );
    case 'join':
      return (
        <ActionButton
          busy={busy === 'join'}
          disabled={busy !== null}
          label={t('subRooms.browse.action.joinRoom', { name })}
          onClick={() => {
            actions.join(room);
          }}
        >
          {t('subRooms.browse.action.join')}
        </ActionButton>
      );
    case 'request':
      return (
        <ActionButton
          aria-expanded={requesting}
          busy={false}
          disabled={busy !== null}
          label={t('subRooms.browse.action.requestRoom', { name })}
          onClick={onRequest}
        >
          {t('subRooms.browse.action.request')}
        </ActionButton>
      );
    case 'pending':
      return (
        <span className="sub-room-browser__pending">
          <span className="sub-room-browser__state">{t('subRooms.browse.action.pending')}</span>
          <ActionButton
            busy={busy === 'withdraw'}
            disabled={busy !== null}
            label={t('subRooms.browse.action.withdrawRoom', { name })}
            onClick={() => {
              actions.withdraw(room);
            }}
          >
            {t('subRooms.browse.action.withdraw')}
          </ActionButton>
        </span>
      );
    case 'invite_only':
      return (
        <span className="sub-room-browser__state">{t('subRooms.browse.action.inviteOnly')}</span>
      );
    case 'banned':
      return null;
  }
}

function Glyph({ room }: { readonly room: SubRoom }) {
  const { t } = useTranslation();
  const gated = room.access !== 'open';
  return (
    <RoomGlyph
      kind={gated ? 'gated' : 'open'}
      {...(gated ? { label: t('subRooms.places.gated') } : {})}
    />
  );
}

function ArchivedRooms({
  actions,
  archived,
  failed,
  parentId,
  principalId,
  query,
}: {
  readonly actions: SubRoomActions;
  readonly archived: SubRoomList | null;
  readonly failed: boolean;
  readonly parentId: string;
  readonly principalId: string | null;
  readonly query: string;
}) {
  const { t } = useTranslation();
  const { privateRooms } = useAppServices();
  const named =
    archived?.subRooms.some(
      (room) =>
        room.archivedByPrincipalId !== undefined && room.archivedByPrincipalId !== principalId,
    ) ?? false;
  // 谁归档的：问父房间里的人叫什么，只在有别人归档的时候问。
  const people = usePrivateRoomPeople(privateRooms, parentId, named);
  if (archived === null)
    return (
      <p className="sub-room-browser__empty" role={failed ? 'alert' : undefined}>
        {failed ? t('subRooms.browse.archivedUnavailable') : t('subRooms.browse.loading')}
      </p>
    );
  const rooms = searchSubRooms(archived.subRooms, query);
  const nameOf = (principal: string): string | null =>
    people.data?.ok === true
      ? (people.data.value.find((person) => person.principalId === principal)?.displayName ?? null)
      : null;
  return (
    <>
      <p className="sub-room-browser__note">{t('subRooms.browse.archivedNote')}</p>
      {rooms.length === 0 ? (
        <p className="sub-room-browser__empty">
          {query.trim() === ''
            ? t('subRooms.browse.empty.archived')
            : t('subRooms.browse.empty.search', { query: query.trim() })}
        </p>
      ) : (
        <ul className="sub-room-browser__rows">
          {rooms.map((room) => (
            <li key={room.catalogId}>
              <ArchivedRoom
                actions={actions}
                category={subRoomCategoryName(archived.categories, room)}
                nameOf={nameOf}
                principalId={principalId}
                room={room}
              />
            </li>
          ))}
        </ul>
      )}
    </>
  );
}

function ArchivedRoom({
  actions,
  category,
  nameOf,
  principalId,
  room,
}: {
  readonly actions: SubRoomActions;
  readonly category: string | null;
  readonly nameOf: (principalId: string) => string | null;
  readonly principalId: string | null;
  readonly room: SubRoom;
}) {
  const { i18n, t } = useTranslation();
  const found = useCatalogUnread(room.catalogId);
  const busy = actions.busy?.catalogId === room.catalogId ? actions.busy.kind : null;
  return (
    <>
      <div className="sub-room-browser__row" data-archived="true">
        <Glyph room={room} />
        <span className="sub-room-browser__name">{room.name}</span>
        <span className="sub-room-browser__purpose">
          {category ?? t('subRooms.browse.uncategorized')}
        </span>
        <span className="sub-room-browser__meta">
          {archivedLine(t, room, principalId, nameOf, i18n.resolvedLanguage)}
        </span>
        <span className="sub-room-browser__action">
          {/* 原来在里面的能打开看；恢复只给管得了它的人。 */}
          {room.joined ? (
            <SubRoomLink
              aria-label={t('subRooms.browse.action.openRoom', { name: room.name })}
              catalogId={room.catalogId}
              className="ar-button ar-button--compact ar-button--ghost"
              matrixRoomId={found?.roomId}
            >
              {t('subRooms.browse.action.open')}
            </SubRoomLink>
          ) : null}
          {room.canRestore === true ? (
            <ActionButton
              busy={busy === 'restore'}
              disabled={busy !== null}
              icon={<RotateCcw aria-hidden="true" />}
              label={t('subRooms.browse.action.restoreRoom', { name: room.name })}
              onClick={() => {
                actions.restore(room);
              }}
            >
              {t('subRooms.browse.action.restore')}
            </ActionButton>
          ) : null}
        </span>
      </div>
      <FailureNote actions={actions} room={room} />
    </>
  );
}

/** 谁在什么时候归档的：自动归档、跟着整间房归档的没有“谁”。 */
function archivedLine(
  t: TFunction,
  room: SubRoom,
  principalId: string | null,
  nameOf: (principalId: string) => string | null,
  language: string | undefined,
): string {
  const date =
    room.archivedAtUnixMs === undefined
      ? ''
      : formatDayAndTime(room.archivedAtUnixMs, Date.now(), language);
  if (room.archivedReason === 'inactive') return t('subRooms.browse.archivedInactive', { date });
  if (room.archivedReason === 'parent') return t('subRooms.browse.archivedWithParent', { date });
  const by = room.archivedByPrincipalId;
  if (by !== undefined && by === principalId) return t('subRooms.browse.archivedByYou', { date });
  const name = by === undefined ? null : nameOf(by);
  return name === null
    ? t('subRooms.browse.archivedBySomeone', { date })
    : t('subRooms.browse.archivedBy', { date, name });
}

/** 申请：留一句话（可以不写，最多 200 字），就在这一行下面，不用再跳一页。没进的那张卡片上也用它。 */
export function RequestForm({
  busy,
  onCancel,
  onSend,
  room,
}: {
  readonly busy: boolean;
  readonly onCancel: () => void;
  readonly onSend: (message: string) => void;
  readonly room: SubRoom;
}) {
  const { t } = useTranslation();
  const messageId = useId();
  const countId = useId();
  const [message, setMessage] = useState('');
  const field = useRef<HTMLTextAreaElement>(null);
  useEffect(() => {
    field.current?.focus();
  }, []);
  // 和服务器一样按字算：表情、汉字都算一个。
  const length = Array.from(message).length;
  const tooLong = length > JOIN_REQUEST_MESSAGE_MAX_LENGTH;
  return (
    <form
      aria-label={t('subRooms.browse.request.label', { name: room.name })}
      className="sub-room-browser__request"
      onSubmit={(event) => {
        event.preventDefault();
        if (!tooLong && !busy) onSend(message);
      }}
    >
      <label htmlFor={messageId}>{t('subRooms.browse.request.message')}</label>
      <textarea
        aria-describedby={countId}
        aria-invalid={tooLong}
        disabled={busy}
        id={messageId}
        onChange={(event) => {
          setMessage(event.target.value);
        }}
        onKeyDown={(event) => {
          if (event.key === 'Escape') onCancel();
        }}
        ref={field}
        rows={2}
        value={message}
      />
      <div className="sub-room-browser__request-footer">
        <small data-over={tooLong ? 'true' : undefined} id={countId}>
          {t('subRooms.browse.request.count', {
            count: length,
            max: JOIN_REQUEST_MESSAGE_MAX_LENGTH,
          })}
        </small>
        <Button disabled={busy} onClick={onCancel} size="compact" tone="ghost" type="button">
          {t('subRooms.browse.request.cancel')}
        </Button>
        <Button
          aria-busy={busy}
          disabled={busy || tooLong}
          icon={
            busy ? (
              <LoaderCircle aria-hidden="true" className="sub-room-browser__spin" />
            ) : undefined
          }
          size="compact"
          type="submit"
        >
          {t('subRooms.browse.request.send')}
        </Button>
      </div>
    </form>
  );
}

function ActionButton({
  busy,
  children,
  disabled,
  icon,
  label,
  onClick,
  ...rest
}: {
  readonly busy: boolean;
  readonly children: string;
  readonly disabled: boolean;
  readonly icon?: ReactNode;
  readonly label: string;
  readonly onClick: () => void;
  readonly 'aria-expanded'?: boolean;
}) {
  return (
    <Button
      {...rest}
      aria-busy={busy}
      aria-label={label}
      disabled={disabled}
      icon={busy ? <LoaderCircle aria-hidden="true" className="sub-room-browser__spin" /> : icon}
      onClick={onClick}
      size="compact"
      tone="ghost"
    >
      {children}
    </Button>
  );
}

/** 说得清的几种没办成；别的说“过一会儿再试”，错误码收在后面。 */
const failureMessages = {
  'sub_room.approval_required': 'subRooms.failure.approval_required',
  'sub_room.archived': 'subRooms.failure.archived',
  'sub_room.banned': 'subRooms.failure.banned',
  'sub_room.invite_only': 'subRooms.failure.invite_only',
  'sub_room.not_in_parent': 'subRooms.failure.not_in_parent',
  'sub_room.rate_limited': 'subRooms.failure.rate_limited',
  'sub_room.recently_rejected': 'subRooms.failure.recently_rejected',
} as const;

function knownFailure(code: string): code is keyof typeof failureMessages {
  return Object.hasOwn(failureMessages, code);
}

/** 刚才在这一间上没办成的那件事：说得清的说，别的说过一会儿再试，错误码跟在后面。 */
export function FailureNote({
  actions,
  room,
}: {
  readonly actions: SubRoomActions;
  readonly room: SubRoom;
}) {
  const { t } = useTranslation();
  const code = actions.failure?.catalogId === room.catalogId ? actions.failure.code : null;
  if (code === null) return null;
  return (
    <p className="sub-room-browser__failure" role="alert">
      {knownFailure(code) ? t(failureMessages[code]) : t('subRooms.failure.other')}
      {knownFailure(code) ? null : <code>{code}</code>}
    </p>
  );
}
