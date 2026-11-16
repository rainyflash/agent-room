import { Button } from '@agent-room/ui-system';
import { useQueryClient } from '@tanstack/react-query';
import { useNavigate } from '@tanstack/react-router';
import {
  Archive,
  Bell,
  BellOff,
  Check,
  CheckCheck,
  ChevronLeft,
  Ellipsis,
  FolderInput,
  LogOut,
  Settings,
} from 'lucide-react';
import { useEffect, useRef, useState, type ReactNode } from 'react';
import { useTranslation } from 'react-i18next';

import { useAppServices } from '@/app/app-services';
import { latestReadingCursor, type RoomUnread } from '@/features/inbox/domain/inbox';
import { RoomDeskMenu } from '@/features/lobby/ui/room-desk-menu';
import { usePersonalWorkspace } from '@/features/personal-workspace/ui/personal-workspace-provider';
import { refreshRoomsAfterClosing } from '@/features/private-rooms/data/private-room-queries';
import type { PrivateRoomFailure } from '@/features/private-rooms/domain/private-room';
import {
  managesSubRoom,
  type SubRoom,
  type SubRoomList,
} from '@/features/private-rooms/domain/sub-rooms';
import { err, type Result } from '@/shared/result';

import './sub-room-row-menu.css';

/** 左栏里每一行“⋯”要用到的、房间页才知道的事。 */
export type SubRoomRowMenuContext = {
  readonly principalId: string;
  /** 父房间的房主：他在每一间子房间里，退不出去，菜单里没有“退出”。不知道时是 null。 */
  readonly ownerPrincipalId: string | null;
  /** 父房间的 Matrix 房间：退出眼前这一间以后回到那儿；不知道就走入场页。 */
  readonly parentMatrixRoomId: string | undefined;
  readonly onOpenSettings: (room: SubRoom) => void;
};

type View = 'main' | 'category' | 'leave' | 'archive';

/** 说得清的几种没办成；别的说“没办成”，后面带着错误码。 */
const failureMessages = {
  'sub_room.owner_stays': 'subRooms.rowMenu.failure.owner_stays',
  'sub_room.archived': 'subRooms.failure.archived',
  'sub_room.not_in_parent': 'subRooms.failure.not_in_parent',
  'sub_room.managers_only': 'subRooms.settings.failure.managers_only',
  'sub_room.category_missing': 'subRooms.create.failure.category_missing',
  'sub_room.rate_limited': 'subRooms.failure.rate_limited',
  personal: 'subRooms.rowMenu.failure.personal',
} as const;

const knownFailure = (code: string): code is keyof typeof failureMessages =>
  Object.hasOwn(failureMessages, code);

const unavailable: PrivateRoomFailure = { code: 'sub_room.unavailable', retryable: false };

/**
 * 左栏里一间子房间那一行的“⋯”（`specs/redesign/sub-rooms.md`“网页”）：标为已读、免打扰对谁都有；
 * 管得了这一间的人还有移到分类、子房间设置、归档；父房间的房主以外的人能退出。
 * 退出和归档先在菜单里问一句，移到分类在菜单里换成分类的名单。
 */
export function SubRoomRowMenu({
  context,
  current,
  found,
  list,
  onNavigate,
  room,
}: {
  readonly context: SubRoomRowMenuContext;
  /** 这一行是不是眼前这一页。 */
  readonly current: boolean;
  /** 收件箱认得的这一间：它的 Matrix 房间和没读的。还没读出来时没有，标为已读和免打扰点不了。 */
  readonly found: { readonly roomId: string; readonly unread: RoomUnread } | undefined;
  readonly list: SubRoomList;
  /** 左栏收起来、在下拉里时把下拉关上。 */
  readonly onNavigate: () => void;
  readonly room: SubRoom;
}) {
  const { t } = useTranslation();
  const anchor = useRef<HTMLButtonElement>(null);
  const [open, setOpen] = useState(false);
  return (
    <>
      <button
        aria-expanded={open}
        aria-haspopup="dialog"
        aria-label={t('subRooms.rowMenu.open', { name: room.name })}
        className="sub-room-places__more"
        onClick={() => {
          setOpen(true);
        }}
        ref={anchor}
        type="button"
      >
        <Ellipsis aria-hidden="true" />
      </button>
      {open ? (
        <RoomDeskMenu
          anchor={anchor}
          label={t('subRooms.rowMenu.open', { name: room.name })}
          onClose={() => {
            setOpen(false);
          }}
        >
          <RowMenuBody
            context={context}
            current={current}
            found={found}
            list={list}
            onClose={() => {
              setOpen(false);
            }}
            onNavigate={onNavigate}
            room={room}
          />
        </RoomDeskMenu>
      ) : null}
    </>
  );
}

function RowMenuBody({
  context,
  current,
  found,
  list,
  onClose,
  onNavigate,
  room,
}: {
  readonly context: SubRoomRowMenuContext;
  readonly current: boolean;
  readonly found: { readonly roomId: string; readonly unread: RoomUnread } | undefined;
  readonly list: SubRoomList;
  readonly onClose: () => void;
  readonly onNavigate: () => void;
  readonly room: SubRoom;
}) {
  const { t } = useTranslation();
  const { messages, subRooms } = useAppServices();
  const personal = usePersonalWorkspace();
  const queries = useQueryClient();
  const navigate = useNavigate();
  const body = useRef<HTMLDivElement>(null);
  const [view, setView] = useState<View>('main');
  const previous = useRef<View>('main');
  const [busy, setBusy] = useState(false);
  const [failure, setFailure] = useState<string | null>(null);

  // 换了一屏：焦点放到新的一屏的第一项；回到主菜单时放回刚才点的那一项。
  useEffect(() => {
    const from = previous.current;
    previous.current = view;
    if (from === view) return;
    const target =
      view === 'main'
        ? body.current?.querySelector<HTMLElement>(`[data-opens="${from}"]`)
        : body.current?.querySelector<HTMLElement>('[data-first]');
    target?.focus();
  }, [view]);

  const roomId = found?.roomId;
  const muted = roomId !== undefined && personal?.snapshot.index.muted.has(roomId) === true;
  const hasUnread =
    found !== undefined && ((found.unread.count ?? 0) > 0 || found.unread.mentioned);
  const manages = managesSubRoom(room);
  const canMove = manages && list.categories.length > 0;
  const canLeave = context.ownerPrincipalId !== context.principalId;
  const ref = { parentId: list.parent.catalogId, subRoomId: room.catalogId };

  const remember = (change: Parameters<NonNullable<typeof personal>['change']>[0]): void => {
    const result = personal?.change(change);
    if (result?.ok === true) onClose();
    else setFailure('personal');
  };
  const markRead = (): void => {
    if (roomId === undefined) return;
    const read = messages.read(roomId);
    const latest = read.ok ? latestReadingCursor(read.value.messages) : null;
    if (latest === null) {
      onClose();
      return;
    }
    remember({ kind: 'read', id: roomId, value: latest });
  };
  const run = (
    action: () => Promise<Result<unknown, PrivateRoomFailure>>,
    done: () => void,
  ): void => {
    setBusy(true);
    setFailure(null);
    void (async () => {
      const result = await action().catch(() => err(unavailable));
      setBusy(false);
      if (!result.ok) {
        setFailure(result.error.code);
        return;
      }
      // 退出、归档以后这一间不在你的房间里了；连子房间的名单（挂在房间列表下面）一起重读。
      await refreshRoomsAfterClosing(queries, room.catalogId);
      done();
    })();
  };
  const leave = (): void => {
    run(
      () => (subRooms === undefined ? Promise.resolve(err(unavailable)) : subRooms.leave(ref)),
      () => {
        onClose();
        if (!current) return;
        // 眼前这一间退出了：回到父房间。
        onNavigate();
        const catalogId = list.parent.catalogId;
        const parentRoom = context.parentMatrixRoomId;
        void (parentRoom === undefined
          ? navigate({ params: { catalogId }, to: '/lobby/$catalogId' })
          : navigate({
              params: { catalogId, roomId: parentRoom },
              to: '/lobby/$catalogId/instance/$roomId',
            }));
      },
    );
  };
  const archive = (): void => {
    run(
      () => (subRooms === undefined ? Promise.resolve(err(unavailable)) : subRooms.archive(ref)),
      onClose,
    );
  };
  const move = (categoryId: string | null): void => {
    if ((room.categoryId ?? null) === categoryId) {
      onClose();
      return;
    }
    run(
      () =>
        subRooms === undefined
          ? Promise.resolve(err(unavailable))
          : subRooms.revise(ref, { categoryId }),
      onClose,
    );
  };

  const failed =
    failure === null ? null : (
      <p className="sub-room-menu__failure" role="alert">
        <span>
          {t(knownFailure(failure) ? failureMessages[failure] : 'subRooms.failure.other')}
        </span>
        {knownFailure(failure) ? null : <code>{failure}</code>}
      </p>
    );

  return (
    <div className="sub-room-menu" ref={body}>
      {view === 'main' ? (
        <>
          <ul className="room-desk-menu__items">
            <Item
              disabled={!hasUnread}
              icon={<CheckCheck aria-hidden="true" />}
              label={t('subRooms.rowMenu.markRead')}
              onPick={markRead}
            />
            <Item
              disabled={roomId === undefined || personal === null}
              hint={muted ? null : t('subRooms.rowMenu.muteNote')}
              icon={muted ? <Bell aria-hidden="true" /> : <BellOff aria-hidden="true" />}
              label={t(muted ? 'subRooms.rowMenu.unmute' : 'subRooms.rowMenu.mute')}
              onPick={() => {
                if (roomId !== undefined) remember({ kind: 'mute', id: roomId, value: !muted });
              }}
            />
          </ul>
          {canMove || manages || canLeave ? (
            <ul className="room-desk-menu__items sub-room-menu__group">
              {canMove ? (
                <Item
                  icon={<FolderInput aria-hidden="true" />}
                  label={t('subRooms.rowMenu.move')}
                  onPick={() => {
                    setFailure(null);
                    setView('category');
                  }}
                  opens="category"
                />
              ) : null}
              {manages ? (
                <Item
                  icon={<Settings aria-hidden="true" />}
                  label={t('subRooms.settings.title')}
                  onPick={() => {
                    onClose();
                    onNavigate();
                    context.onOpenSettings(room);
                  }}
                  opensDialog
                />
              ) : null}
              {canLeave ? (
                <Item
                  icon={<LogOut aria-hidden="true" />}
                  label={t('subRooms.rowMenu.leave')}
                  onPick={() => {
                    setFailure(null);
                    setView('leave');
                  }}
                  opens="leave"
                />
              ) : null}
              {manages ? (
                <Item
                  icon={<Archive aria-hidden="true" />}
                  label={t('subRooms.rowMenu.archive')}
                  onPick={() => {
                    setFailure(null);
                    setView('archive');
                  }}
                  opens="archive"
                />
              ) : null}
            </ul>
          ) : null}
          {failed}
        </>
      ) : view === 'category' ? (
        <>
          <Back
            label={t('subRooms.rowMenu.moveTitle', { name: room.name })}
            onBack={() => {
              setView('main');
            }}
          />
          <ul className="room-desk-menu__items">
            {[
              ...[...list.categories]
                .sort((left, right) => left.position - right.position)
                .map((category) => ({ id: category.categoryId, name: category.name })),
              { id: null, name: t('subRooms.browse.uncategorized') },
            ].map((category, index) => {
              const here = (room.categoryId ?? null) === category.id;
              return (
                <li key={category.id ?? ''}>
                  <button
                    aria-pressed={here}
                    className="room-desk-menu__item"
                    data-first={index === 0 ? '' : undefined}
                    disabled={busy}
                    onClick={() => {
                      move(category.id);
                    }}
                    type="button"
                  >
                    <span>{category.name}</span>
                    {here ? <Check aria-hidden="true" className="sub-room-menu__here" /> : null}
                  </button>
                </li>
              );
            })}
          </ul>
          {failed}
        </>
      ) : (
        <Confirm
          busy={busy}
          confirm={t(view === 'leave' ? 'subRooms.rowMenu.leave' : 'subRooms.rowMenu.archive')}
          detail={
            view === 'leave'
              ? t(`subRooms.rowMenu.leaveDetail.${room.access}`)
              : t('subRooms.settings.archiveDetail')
          }
          failed={failed}
          onCancel={() => {
            setView('main');
          }}
          onConfirm={view === 'leave' ? leave : archive}
          title={t(
            view === 'leave' ? 'subRooms.rowMenu.leaveTitle' : 'subRooms.rowMenu.archiveTitle',
            { name: room.name },
          )}
        />
      )}
    </div>
  );
}

function Item({
  disabled = false,
  hint = null,
  icon,
  label,
  onPick,
  opens,
  opensDialog = false,
}: {
  readonly disabled?: boolean;
  readonly hint?: string | null;
  readonly icon: ReactNode;
  readonly label: string;
  readonly onPick: () => void;
  /** 点了在菜单里换到哪一屏；回来时焦点放回这一项。 */
  readonly opens?: View;
  readonly opensDialog?: boolean;
}) {
  return (
    <li>
      <button
        aria-haspopup={opensDialog ? 'dialog' : undefined}
        className="room-desk-menu__item"
        data-opens={opens}
        disabled={disabled}
        onClick={onPick}
        type="button"
      >
        {icon}
        <span className="sub-room-menu__label">
          <span>{label}</span>
          {hint === null ? null : <small>{hint}</small>}
        </span>
      </button>
    </li>
  );
}

function Back({ label, onBack }: { readonly label: string; readonly onBack: () => void }) {
  const { t } = useTranslation();
  return (
    <div className="sub-room-menu__head">
      <button
        aria-label={t('subRooms.rowMenu.back')}
        className="sub-room-menu__back"
        onClick={onBack}
        type="button"
      >
        <ChevronLeft aria-hidden="true" />
      </button>
      <strong>{label}</strong>
    </div>
  );
}

function Confirm({
  busy,
  confirm,
  detail,
  failed,
  onCancel,
  onConfirm,
  title,
}: {
  readonly busy: boolean;
  readonly confirm: string;
  readonly detail: string;
  readonly failed: ReactNode;
  readonly onCancel: () => void;
  readonly onConfirm: () => void;
  readonly title: string;
}) {
  const { t } = useTranslation();
  return (
    <div aria-label={title} className="sub-room-menu__confirm" role="group">
      <strong>{title}</strong>
      <p>{detail}</p>
      {failed}
      <div className="sub-room-menu__actions">
        <Button data-first="" disabled={busy} onClick={onCancel} size="compact" tone="quiet">
          {t('subRooms.rowMenu.cancel')}
        </Button>
        <Button disabled={busy} onClick={onConfirm} size="compact" tone="alert">
          {confirm}
        </Button>
      </div>
    </div>
  );
}
