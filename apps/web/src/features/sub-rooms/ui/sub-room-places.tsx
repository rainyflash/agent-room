import { RoomGlyph } from '@agent-room/ui-system';
import type { TFunction } from 'i18next';
import { ChevronDown, List, Plus } from 'lucide-react';
import { useId, useState, type ReactNode } from 'react';
import { useTranslation } from 'react-i18next';

import { useCatalogUnread } from '@/features/inbox/ui/room-unread-mark';
import type { SubRoom, SubRoomList } from '@/features/private-rooms/domain/sub-rooms';
import {
  subRoomPlaceGroups,
  type SubRoomPlaceGroup,
} from '@/features/sub-rooms/domain/sub-room-places';
import { SubRoomLink } from './sub-room-link';
import './sub-room-places.css';

export type SubRoomPlacesProps = {
  readonly list: SubRoomList;
  /** 眼前这一页的房间号：是子房间时它那一行是“你在这儿”。 */
  readonly currentCatalogId: string;
  /** 人是不是在这一页自己的对话里（不在私聊里、不在看展开的那一页）。 */
  readonly currentSelected: boolean;
  /** 人不在这一页的对话里时，这里来了几条他还没看到的。 */
  readonly currentUnread: number;
  readonly onSelectCurrent: () => void;
  /** 在下拉里点了一项要先把下拉关上；左栏里什么都不做。 */
  readonly onNavigate: () => void;
  /** 最上面“浏览子房间”那一行；这间房下面一间都没有、你也建不了时没有。 */
  readonly browse: SubRoomBrowseRow | null;
  /** 最下面“新建子房间”那一行；你建不了时为空。 */
  readonly onCreate: (() => void) | null;
};

export type SubRoomBrowseRow = {
  /** 名单上有几间（不算归档的）。 */
  readonly count: number;
  /** 有几间你没进、这台设备上又没见过：有就带一个点。 */
  readonly unseen: number;
  /** “浏览子房间”这一页正开着。 */
  readonly selected: boolean;
  /** 眼前是父房间：点了打开这一页。眼前是子房间时为空，这一行去父房间的这一页。 */
  readonly onOpen: (() => void) | null;
  /** 父房间的 Matrix 房间，去父房间时用；不知道就走入场页。 */
  readonly parentMatrixRoomId: string | undefined;
};

/**
 * 左栏里的子房间（`specs/redesign/sub-rooms.md`“网页”）：你在的那几间按分类分组，分类能收起。
 * 能直接进的前面是 `#`，有门的是锁；提到你的是红底的数，没读的是灰色的数。
 */
export function SubRoomPlaces(props: SubRoomPlacesProps) {
  const { t } = useTranslation();
  const groups = subRoomPlaceGroups(props.list, props.currentCatalogId);
  const [collapsed, setCollapsed] = useState<ReadonlySet<string>>(() =>
    readCollapsed(props.list.parent.catalogId),
  );
  if (groups.length === 0 && props.browse === null && props.onCreate === null) return null;
  const toggle = (categoryId: string): void => {
    const next = new Set(collapsed);
    if (!next.delete(categoryId)) next.add(categoryId);
    setCollapsed(next);
    writeCollapsed(props.list.parent.catalogId, next);
  };
  return (
    <nav aria-label={t('subRooms.places.label')} className="sub-room-places">
      {props.browse === null ? null : <BrowseRow props={props} row={props.browse} />}
      {groups.map((group) =>
        group.category === null ? (
          <PlaceList group={group} key="uncategorized" props={props} />
        ) : (
          <CategoryGroup
            closed={collapsed.has(group.category.categoryId)}
            group={group}
            key={group.category.categoryId}
            onToggle={() => {
              if (group.category !== null) toggle(group.category.categoryId);
            }}
            props={props}
          />
        ),
      )}
      {props.onCreate === null ? null : <CreateRow onCreate={props.onCreate} props={props} />}
    </nav>
  );
}

/** “+ 新建子房间”：打开新建的对话框，建好就进去。 */
function CreateRow({
  onCreate,
  props,
}: {
  readonly onCreate: () => void;
  readonly props: SubRoomPlacesProps;
}) {
  const { t } = useTranslation();
  return (
    <button
      className="room-desk__place sub-room-places__create"
      onClick={() => {
        props.onNavigate();
        onCreate();
      }}
      type="button"
    >
      <Plus aria-hidden="true" className="sub-room-places__browse-icon" />
      <span className="sub-room-places__name">{t('subRooms.create.open')}</span>
    </button>
  );
}

/** “浏览子房间”：后面是有几间，有没见过的新的带一个点。 */
function BrowseRow({
  props,
  row,
}: {
  readonly props: SubRoomPlacesProps;
  readonly row: SubRoomBrowseRow;
}) {
  const { t } = useTranslation();
  const freshId = useId();
  const body = (
    <>
      <List aria-hidden="true" className="sub-room-places__browse-icon" />
      <span className="sub-room-places__name">{t('subRooms.places.browse')}</span>
      {row.unseen > 0 ? (
        <span className="sub-room-places__fresh" id={freshId}>
          <span className="sr-only">{t('subRooms.places.unseen', { count: row.unseen })}</span>
        </span>
      ) : null}
      <span aria-hidden="true" className="sub-room-places__total">
        {row.count}
      </span>
    </>
  );
  const shared = {
    'aria-describedby': row.unseen > 0 ? freshId : undefined,
    'aria-label': t('subRooms.places.browseCount', { count: row.count }),
    className: 'room-desk__place sub-room-places__browse',
  };
  if (row.onOpen !== null) {
    const open = row.onOpen;
    return (
      <button
        {...shared}
        aria-current={row.selected ? 'page' : undefined}
        onClick={() => {
          props.onNavigate();
          open();
        }}
        type="button"
      >
        {body}
      </button>
    );
  }
  return (
    <SubRoomLink
      {...shared}
      catalogId={props.list.parent.catalogId}
      matrixRoomId={row.parentMatrixRoomId}
      onClick={props.onNavigate}
      search={{ view: 'sub-rooms' }}
    >
      {body}
    </SubRoomLink>
  );
}

function CategoryGroup({
  closed,
  group,
  onToggle,
  props,
}: {
  readonly closed: boolean;
  readonly group: SubRoomPlaceGroup;
  readonly onToggle: () => void;
  readonly props: SubRoomPlacesProps;
}) {
  const listId = useId();
  // 收起来的分类里，眼前这一间照样留着：不然人看不出自己在哪儿。
  const shown = closed
    ? { ...group, rooms: group.rooms.filter((room) => room.catalogId === props.currentCatalogId) }
    : group;
  return (
    <section className="sub-room-places__group">
      <h2 className="sub-room-places__heading">
        <button
          aria-controls={listId}
          aria-expanded={!closed}
          className="sub-room-places__category"
          onClick={onToggle}
          type="button"
        >
          <ChevronDown aria-hidden="true" />
          <span>{group.category?.name}</span>
        </button>
      </h2>
      <div id={listId}>
        <PlaceList group={shown} props={props} />
      </div>
    </section>
  );
}

function PlaceList({
  group,
  props,
}: {
  readonly group: SubRoomPlaceGroup;
  readonly props: SubRoomPlacesProps;
}) {
  if (group.rooms.length === 0) return null;
  return (
    <ul className="sub-room-places__rooms">
      {group.rooms.map((room) => (
        <li key={room.catalogId}>
          <SubRoomPlace props={props} room={room} />
        </li>
      ))}
    </ul>
  );
}

function SubRoomPlace({
  props,
  room,
}: {
  readonly props: SubRoomPlacesProps;
  readonly room: SubRoom;
}) {
  const { t } = useTranslation();
  const badgeId = useId();
  const doorId = useId();
  const gated = room.access !== 'open';
  const found = useCatalogUnread(room.catalogId);
  const current = room.catalogId === props.currentCatalogId;
  // 眼前这一间：人在它的对话里时没有没读的；在私聊里时照这一页自己数的。
  const count = current
    ? props.currentSelected
      ? 0
      : props.currentUnread
    : (found?.unread.count ?? 0);
  const mentioned = !current && (found?.unread.mentioned ?? false);
  const badge = unreadBadge(count, mentioned, badgeId, t);
  const body = (
    <>
      <RoomGlyph kind={gated ? 'gated' : 'open'} />
      <span className="sub-room-places__name">{room.name}</span>
      {gated ? (
        <span className="sr-only" id={doorId}>
          {t('subRooms.places.gated')}
        </span>
      ) : null}
      {badge}
    </>
  );
  // 名字就是房间名；有门、没读的经 `aria-describedby` 读出来。
  const described = [gated ? doorId : null, badge === null ? null : badgeId].filter(
    (id) => id !== null,
  );
  const shared = {
    'aria-describedby': described.length === 0 ? undefined : described.join(' '),
    className: 'room-desk__place sub-room-places__room',
    'data-unread': count > 0 || mentioned ? ('true' as const) : undefined,
    title: room.description === '' ? room.name : `${room.name}\n${room.description}`,
  };
  if (current) {
    return (
      <button
        {...shared}
        aria-current={props.currentSelected ? 'page' : undefined}
        aria-label={room.name}
        onClick={() => {
          props.onNavigate();
          props.onSelectCurrent();
        }}
        type="button"
      >
        {body}
      </button>
    );
  }
  return (
    <SubRoomLink
      {...shared}
      aria-label={room.name}
      catalogId={room.catalogId}
      matrixRoomId={found?.roomId}
      onClick={props.onNavigate}
    >
      {body}
    </SubRoomLink>
  );
}

function unreadBadge(count: number, mentioned: boolean, id: string, t: TFunction): ReactNode {
  if (!mentioned && count <= 0) return null;
  const shown = count > 99 ? '99+' : String(count);
  const spoken = [
    mentioned ? t('subRooms.places.mentioned') : null,
    count > 0 ? t('subRooms.places.unread', { count }) : null,
  ]
    .filter((part) => part !== null)
    .join(', ');
  return (
    <span className={mentioned ? 'sub-room-places__mention' : 'sub-room-places__count'} id={id}>
      <span aria-hidden="true">{count > 0 ? shown : '@'}</span>
      <span className="sr-only">{spoken}</span>
    </span>
  );
}

const COLLAPSED_KEY = 'agent-room.sub-room-categories.collapsed.';

/** 收起了哪些分类只记在这台设备上，按父房间分开；读不了就当都展开着。 */
function readCollapsed(parentId: string): ReadonlySet<string> {
  try {
    const raw = window.localStorage.getItem(COLLAPSED_KEY + parentId);
    const parsed: unknown = raw === null ? [] : JSON.parse(raw);
    return new Set(
      Array.isArray(parsed) ? parsed.filter((id): id is string => typeof id === 'string') : [],
    );
  } catch {
    return new Set();
  }
}

function writeCollapsed(parentId: string, collapsed: ReadonlySet<string>): void {
  try {
    window.localStorage.setItem(COLLAPSED_KEY + parentId, JSON.stringify([...collapsed]));
  } catch {
    // 存不下就只在这一次打开里记着。
  }
}
