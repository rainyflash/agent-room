import { Avatar, Banner, Button, Field } from '@agent-room/ui-system';
import { useMutation, useQueryClient } from '@tanstack/react-query';
import { Plus } from 'lucide-react';
import { useEffect, useId, useRef, useState } from 'react';
import { useTranslation } from 'react-i18next';

import { useAppServices } from '@/app/app-services';
import {
  privateRoomListQueryKey,
  usePrivateRoomPeople,
} from '@/features/private-rooms/data/private-room-queries';
import {
  allows,
  isRoomAdmin,
  memberFor,
  type PrivateRoom,
} from '@/features/private-rooms/domain/private-room';
import type { SubRoom, SubRoomRef } from '@/features/private-rooms/domain/sub-rooms';
import { err } from '@/shared/result';

import './sub-room-managers.css';

/** 服务器说这件事要最近登录过：和别的要重新登录的操作同一个码。 */
const REAUTHENTICATION_REQUIRED = 'authentication.reauthentication_required';

/** 说得清的几种没办成；别的说“没办成”，错误码跟在后面。 */
const failureMessages = {
  'sub_room.archived': 'subRooms.failure.archived',
  'sub_room.managed_by_parent': 'subRooms.settings.managers.failure.managed_by_parent',
  'sub_room.managers_only': 'subRooms.settings.failure.managers_only',
  'sub_room.not_in_parent': 'subRooms.settings.requests.failure.not_in_parent',
  'sub_room.peer_manager': 'subRooms.settings.managers.failure.peer_manager',
  'sub_room.target_not_inside': 'subRooms.settings.managers.failure.target_not_inside',
} as const;

function knownFailure(code: string): code is keyof typeof failureMessages {
  return Object.hasOwn(failureMessages, code);
}

/**
 * 这个人靠整间房管着每一间：房主（子房间的房主就是父房间的房主）和父房间的管理员。父房间不在手边时，
 * 按这一间里身上有“管理”、又不在子房间管理员名单里的算（“管理”只给这两种人）。
 */
export function runsEverySubRoom(
  principalId: string,
  room: SubRoom,
  members: PrivateRoom,
  parent: PrivateRoom | null,
): boolean {
  if (principalId === members.ownerPrincipalId) return true;
  if (parent !== null) {
    const member = memberFor(parent, principalId);
    return member !== undefined && isRoomAdmin(parent, member);
  }
  return (
    allows(memberFor(members, principalId), 'manage') &&
    !room.managerPrincipalIds.includes(principalId)
  );
}

export type SubRoomManagersProps = {
  readonly subRoomRef: SubRoomRef;
  readonly room: SubRoom;
  /** 这一间在房间设置里读到的样子：房主是谁、里面有谁、谁身上有“管理”。 */
  readonly members: PrivateRoom;
  /** 父房间：管理员是谁。房间列表里没有时为空。 */
  readonly parent: PrivateRoom | null;
  readonly principalId: string;
  /** 设别人、撤别人要最近登录过（自己不当了不用）：不是的话请人重新登录。 */
  readonly recentlyAuthenticated: boolean;
  readonly onReauthenticate: () => void;
};

type Change = { readonly kind: 'grant' | 'revoke'; readonly principalId: string };

/**
 * 子房间设置里“管这一间的”（画布第 8 张）：子房间管理员一行一个，建的人、房主和管理员标出来；
 * 管得了这一间的人能从这一间里的人里再设一个。撤别人要父房间的房主或管理员，子房间管理员只能自己不当了
 * （`specs/redesign/sub-rooms.md` 的“子房间管理员”）。
 */
export function SubRoomManagers({
  members,
  onReauthenticate,
  parent,
  principalId,
  recentlyAuthenticated,
  room,
  subRoomRef,
}: SubRoomManagersProps) {
  const { t } = useTranslation();
  const { privateRooms, subRooms } = useAppServices();
  const queryClient = useQueryClient();
  const headingId = useId();
  const [adding, setAdding] = useState(false);
  const [pick, setPick] = useState('');
  const [signIn, setSignIn] = useState(false);
  const [steppingDown, setSteppingDown] = useState(false);
  const pickRef = useRef<HTMLSelectElement>(null);
  const addButton = useRef<HTMLButtonElement>(null);
  // 名字从父房间的成员里认：管父房间的人不一定进了这一间，父房间的谁都问得到。
  const people = usePrivateRoomPeople(privateRooms, subRoomRef.parentId);
  const names = new Map(
    people.data?.ok === true
      ? people.data.value.map((person) => [person.principalId, person.displayName.trim()])
      : [],
  );
  const nameOf = (id: string): string => {
    const name = names.get(id);
    return name === undefined || name === '' ? t('subRooms.settings.managers.someone') : name;
  };
  const change = useMutation({
    mutationFn: async ({ kind, principalId: target }: Change) => {
      if (subRooms === undefined) return err({ code: 'sub_room.unavailable', retryable: false });
      return kind === 'grant'
        ? await subRooms.grantManager(subRoomRef, target)
        : await subRooms.revokeManager(subRoomRef, target);
    },
    onSuccess: (result) => {
      if (!result.ok) return;
      setAdding(false);
      setPick('');
      setSteppingDown(false);
    },
    onSettled: async () => {
      // 名单上的“谁在管”、这一间身上的“管理”和身份变动都挂在房间列表的键下面，一起重读。
      await queryClient.invalidateQueries({ queryKey: privateRoomListQueryKey });
    },
  });
  useEffect(() => {
    if (adding) pickRef.current?.focus();
  }, [adding]);

  const viewerRunsEvery = runsEverySubRoom(principalId, room, members, parent);
  const candidates = members.members.filter(
    (member) =>
      member.status === 'joined' &&
      !room.managerPrincipalIds.includes(member.principalId) &&
      !runsEverySubRoom(member.principalId, room, members, parent),
  );
  const failure = change.data?.ok === false ? change.data.error.code : null;
  const busy = change.isPending;
  // 设别人、撤别人前先看是不是最近登录过；服务器说不算了也照这样请人重新登录。
  const needsSignIn = signIn || failure === REAUTHENTICATION_REQUIRED;
  const guarded = (next: () => void) => {
    if (!recentlyAuthenticated) {
      setSignIn(true);
      return;
    }
    setSignIn(false);
    next();
  };

  return (
    <section aria-labelledby={headingId} className="sub-room-managers">
      <div className="sub-room-managers__head">
        <h3 id={headingId}>{t('subRooms.settings.managers.title')}</h3>
        {candidates.length === 0 || adding ? null : (
          <Button
            disabled={busy}
            icon={<Plus aria-hidden="true" />}
            onClick={() => {
              guarded(() => {
                setAdding(true);
                setPick('');
              });
            }}
            ref={addButton}
            size="compact"
            tone="quiet"
          >
            {t('subRooms.settings.managers.add')}
          </Button>
        )}
      </div>
      {needsSignIn ? (
        <Banner
          action={
            <Button onClick={onReauthenticate} size="compact" tone="quiet">
              {t('privateRooms.governance.members.signInAgain')}
            </Button>
          }
          tone="warning"
        >
          {t('privateRooms.governance.members.signInFirst')}
        </Banner>
      ) : null}
      {adding ? (
        <form
          className="sub-room-managers__pick"
          onSubmit={(event) => {
            event.preventDefault();
            if (pick === '' || busy) return;
            change.mutate({ kind: 'grant', principalId: pick });
          }}
        >
          <Field
            hint={t('subRooms.settings.managers.pickHint')}
            label={t('subRooms.settings.managers.who')}
          >
            <select
              disabled={busy}
              onChange={(event) => {
                setPick(event.target.value);
              }}
              ref={pickRef}
              value={pick}
            >
              <option disabled value="">
                {t('subRooms.settings.managers.choose')}
              </option>
              {candidates.map((member) => (
                <option key={member.principalId} value={member.principalId}>
                  {nameOf(member.principalId)}
                </option>
              ))}
            </select>
          </Field>
          <div className="sub-room-managers__pick-actions">
            <Button
              disabled={busy}
              onClick={() => {
                setAdding(false);
                addButton.current?.focus();
              }}
              size="compact"
              tone="quiet"
            >
              {t('subRooms.settings.managers.cancel')}
            </Button>
            <Button aria-busy={busy} disabled={pick === '' || busy} size="compact" type="submit">
              {t('subRooms.settings.managers.make')}
            </Button>
          </div>
        </form>
      ) : null}
      {room.managerPrincipalIds.length === 0 ? (
        <p className="sub-room-managers__empty">{t('subRooms.settings.managers.none')}</p>
      ) : (
        <ul className="sub-room-managers__list">
          {room.managerPrincipalIds.map((id) => {
            const self = id === principalId;
            const throughRoom = runsEverySubRoom(id, room, members, parent);
            const notes = [
              self ? t('subRooms.settings.managers.you') : null,
              id === members.ownerPrincipalId
                ? t('subRooms.settings.managers.owner')
                : throughRoom
                  ? t('subRooms.settings.managers.admin')
                  : null,
              id === room.createdByPrincipalId ? t('subRooms.settings.managers.creator') : null,
            ].filter((note) => note !== null);
            const name = nameOf(id);
            return (
              <li key={id}>
                <Avatar tint="ink">{(Array.from(name)[0] ?? '?').toUpperCase()}</Avatar>
                <div className="sub-room-managers__who">
                  <strong>{name}</strong>
                  {notes.length === 0 ? null : <small>{notes.join(' · ')}</small>}
                </div>
                {/* 房主和管理员本来就管每一间，名单上的这一条撤不撤都一样，不给按钮。 */}
                {throughRoom ? null : self ? (
                  <Button
                    aria-expanded={steppingDown}
                    disabled={busy}
                    onClick={() => {
                      setSteppingDown((open) => !open);
                    }}
                    size="compact"
                    tone="quiet"
                  >
                    {t('subRooms.settings.managers.stepDown')}
                  </Button>
                ) : viewerRunsEvery ? (
                  <Button
                    aria-label={t('subRooms.settings.managers.removeLabel', { name })}
                    disabled={busy}
                    onClick={() => {
                      guarded(() => {
                        change.mutate({ kind: 'revoke', principalId: id });
                      });
                    }}
                    size="compact"
                    tone="quiet"
                  >
                    {t('subRooms.settings.managers.remove')}
                  </Button>
                ) : null}
                {self && steppingDown && !throughRoom ? (
                  <div
                    aria-label={t('subRooms.settings.managers.stepDownTitle', { name: room.name })}
                    className="sub-room-managers__confirm"
                    role="group"
                  >
                    <p>{t('subRooms.settings.managers.stepDownDetail')}</p>
                    <div className="sub-room-managers__pick-actions">
                      <Button
                        disabled={busy}
                        onClick={() => {
                          setSteppingDown(false);
                        }}
                        size="compact"
                        tone="quiet"
                      >
                        {t('subRooms.settings.managers.cancel')}
                      </Button>
                      <Button
                        aria-busy={busy}
                        disabled={busy}
                        onClick={() => {
                          // 自己不当了不用最近登录过。
                          change.mutate({ kind: 'revoke', principalId: id });
                        }}
                        size="compact"
                      >
                        {t('subRooms.settings.managers.stepDown')}
                      </Button>
                    </div>
                  </div>
                ) : null}
              </li>
            );
          })}
        </ul>
      )}
      {failure === null || failure === REAUTHENTICATION_REQUIRED ? null : (
        <p className="sub-room-managers__failure" role="alert">
          {knownFailure(failure) ? t(failureMessages[failure]) : t('subRooms.failure.other')}
          {knownFailure(failure) ? null : <code>{failure}</code>}
        </p>
      )}
    </section>
  );
}
