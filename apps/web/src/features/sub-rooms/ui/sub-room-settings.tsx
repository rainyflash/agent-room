import { Banner, Button, Details, Field } from '@agent-room/ui-system';
import { useQueryClient } from '@tanstack/react-query';
import { Archive, Check, LoaderCircle } from 'lucide-react';
import { useId, useRef, useState } from 'react';
import { useTranslation } from 'react-i18next';

import { useAppServices } from '@/app/app-services';
import {
  privateRoomListQueryKey,
  refreshRoomsAfterClosing,
} from '@/features/private-rooms/data/private-room-queries';
import {
  subRoomAccessValues,
  type ReviseSubRoomInput,
  type SubRoom,
  type SubRoomAccess,
  type SubRoomList,
} from '@/features/private-rooms/domain/sub-rooms';
import { ArchiveRoomConfirm } from '@/features/private-rooms/ui/archive-room-confirm';

import { SubRoomManagers, type SubRoomManagersProps } from './sub-room-managers';
import { SubRoomWaiting } from './sub-room-waiting';

import '@/features/private-rooms/ui/private-room-public-settings.css';
import './sub-room-settings.css';

const NAME_LIMIT = 128;
const PURPOSE_LIMIT = 2_048;

/** 说得清的几种没存上；别的说“没存上”，错误码收在“详情”里。 */
const failureMessages = {
  'private_room.conflict': 'subRooms.settings.failure.conflict',
  'sub_room.archived': 'subRooms.failure.archived',
  'sub_room.category_missing': 'subRooms.create.failure.category_missing',
  'sub_room.managers_only': 'subRooms.settings.failure.managers_only',
  'sub_room.name_taken': 'subRooms.create.failure.name_taken',
} as const;

function knownFailure(code: string): code is keyof typeof failureMessages {
  return Object.hasOwn(failureMessages, code);
}

type Draft = {
  readonly name: string;
  readonly purpose: string;
  /** 空字符串是“未分类”。 */
  readonly categoryId: string;
  readonly access: SubRoomAccess;
  readonly hidden: boolean;
};

const draftOf = (room: SubRoom): Draft => ({
  name: room.name,
  purpose: room.description,
  categoryId: room.categoryId ?? '',
  access: room.access,
  hidden: room.hidden,
});

/**
 * 只带改了的那几项（服务器没带的不动）。拆门时不提隐藏：服务器跟着去掉，明说要藏反倒回 400。
 * 什么都没改是 `null`。
 */
export function subRoomRevision(room: SubRoom, draft: Draft): ReviseSubRoomInput | null {
  const name = draft.name.trim();
  const description = draft.purpose.trim();
  const categoryId = draft.categoryId === '' ? null : draft.categoryId;
  const hidden = draft.access !== 'open' && draft.hidden;
  const input: ReviseSubRoomInput = {
    ...(name === room.name ? {} : { name }),
    ...(description === room.description ? {} : { description }),
    ...(categoryId === (room.categoryId ?? null) ? {} : { categoryId }),
    ...(draft.access === room.access ? {} : { access: draft.access }),
    ...(draft.access === 'open' || hidden === room.hidden ? {} : { hidden }),
  };
  return Object.keys(input).length === 0 ? null : input;
}

export type SubRoomSettingsProps = {
  /** 这间子房间所在的整间房的名单：父房间和分类从这里来。 */
  readonly list: SubRoomList;
  readonly room: SubRoom;
  /** 归档成了：房间页接着照归档的子房间显示。 */
  readonly onArchived: () => void;
  /** “管这一间的”要的东西（这一间读到的样子、父房间、你是谁）；还没读到这一间时为空，那一块先不出现。 */
  readonly managing?: Omit<SubRoomManagersProps, 'room' | 'subRoomRef'> | null | undefined;
};

/**
 * 子房间设置（`specs/redesign/sub-rooms.md`“网页”，画布第 8 张）：最上面在等的申请和敲门，名字、用途、分类，
 * 谁能进和“不在名单里显示”，最下面是归档。这一间的管理员、父房间的房主和管理员看得到。管这一间的人在下一步加。
 */
export function SubRoomSettings({ list, managing = null, onArchived, room }: SubRoomSettingsProps) {
  const { t } = useTranslation();
  const { subRooms } = useAppServices();
  const queryClient = useQueryClient();
  const formId = useId();
  const [draft, setDraft] = useState<Draft>(() => draftOf(room));
  const [saving, setSaving] = useState(false);
  const [saved, setSaved] = useState(false);
  const [failure, setFailure] = useState<string | null>(null);
  const [confirmingArchive, setConfirmingArchive] = useState(false);
  const [archiving, setArchiving] = useState(false);
  const [archiveFailure, setArchiveFailure] = useState<string | null>(null);
  const archiveButton = useRef<HTMLButtonElement>(null);
  const ref = { parentId: list.parent.catalogId, subRoomId: room.catalogId };
  const nameError = draft.name.trim().includes('/') ? t('subRooms.create.nameSlash') : undefined;
  const revision = subRoomRevision(room, draft);
  const ready = revision !== null && draft.name.trim().length > 0 && nameError === undefined;
  const busy = saving || archiving;
  const change = (next: Partial<Draft>) => {
    setDraft((current) => ({ ...current, ...next }));
    setSaved(false);
    setFailure(null);
  };
  const save = async () => {
    if (!ready || busy) return;
    setSaving(true);
    setFailure(null);
    const result =
      subRooms === undefined ? null : await subRooms.revise(ref, revision).catch(() => null);
    setSaving(false);
    if (result?.ok !== true) {
      setFailure(result?.error.code ?? 'sub_room.unavailable');
      // 别人刚换过门：名单重问一遍，“从现在的样子重来”拿得到新的。
      if (result?.error.code === 'private_room.conflict')
        await queryClient.invalidateQueries({ queryKey: privateRoomListQueryKey });
      return;
    }
    setSaved(true);
    await queryClient.invalidateQueries({ queryKey: privateRoomListQueryKey });
  };
  const archive = async () => {
    setArchiving(true);
    setArchiveFailure(null);
    const result = subRooms === undefined ? null : await subRooms.archive(ref).catch(() => null);
    setArchiving(false);
    if (result?.ok !== true) {
      setArchiveFailure(result?.error.code ?? 'sub_room.unavailable');
      return;
    }
    await refreshRoomsAfterClosing(queryClient, room.catalogId);
    onArchived();
  };
  const addsDoor = room.access === 'open' && draft.access !== 'open';
  const removesDoor = room.access !== 'open' && draft.access === 'open';
  const voidedRequests =
    room.access === 'request' &&
    (draft.access !== 'request' || (draft.hidden && !room.hidden)) &&
    (room.pendingRequestCount ?? 0) > 0
      ? (room.pendingRequestCount ?? 0)
      : 0;
  return (
    <div className="sub-room-settings">
      <SubRoomWaiting room={room} subRoomRef={ref} />
      <form
        className="sub-room-settings__form"
        id={formId}
        onSubmit={(event) => {
          event.preventDefault();
          void save();
        }}
      >
        <div
          className="sub-room-settings__row"
          data-categories={list.categories.length === 0 ? undefined : 'true'}
        >
          <Field error={nameError} label={t('subRooms.create.name')}>
            <input
              disabled={busy}
              maxLength={NAME_LIMIT}
              onChange={(event) => {
                change({ name: event.target.value });
              }}
              required
              value={draft.name}
            />
          </Field>
          {list.categories.length === 0 ? null : (
            <Field label={t('subRooms.create.category')}>
              <select
                disabled={busy}
                onChange={(event) => {
                  change({ categoryId: event.target.value });
                }}
                value={draft.categoryId}
              >
                <option value="">{t('subRooms.browse.uncategorized')}</option>
                {[...list.categories]
                  .sort((a, b) => a.position - b.position)
                  .map((category) => (
                    <option key={category.categoryId} value={category.categoryId}>
                      {category.name}
                    </option>
                  ))}
              </select>
            </Field>
          )}
        </div>
        <Field hint={t('subRooms.create.purposeHint')} label={t('subRooms.create.purpose')}>
          <textarea
            disabled={busy}
            maxLength={PURPOSE_LIMIT}
            onChange={(event) => {
              change({ purpose: event.target.value });
            }}
            rows={2}
            value={draft.purpose}
          />
        </Field>
        <fieldset className="room-listing__choices room-listing__choices--three" disabled={busy}>
          <legend>{t('subRooms.create.access')}</legend>
          {subRoomAccessValues.map((value) => (
            <label key={value}>
              <input
                checked={draft.access === value}
                name={`sub-room-settings-access-${formId}`}
                onChange={() => {
                  change({ access: value });
                }}
                type="radio"
                value={value}
              />
              <span>
                <strong>{t(`subRooms.create.accessChoice.${value}.label`)}</strong>
                <small>{t(`subRooms.create.accessChoice.${value}.detail`)}</small>
              </span>
            </label>
          ))}
        </fieldset>
        <label className="sub-room-settings__hidden">
          <input
            checked={draft.access !== 'open' && draft.hidden}
            disabled={busy || draft.access === 'open'}
            onChange={(event) => {
              change({ hidden: event.target.checked });
            }}
            type="checkbox"
          />
          <span>
            <span>{t('subRooms.settings.hidden')}</span>
            <small>
              {t(
                draft.access === 'open'
                  ? 'subRooms.settings.hiddenNeedsDoor'
                  : 'subRooms.settings.hiddenDetail',
              )}
            </small>
          </span>
        </label>
        {addsDoor || removesDoor || voidedRequests > 0 ? (
          <ul className="sub-room-settings__effects">
            {addsDoor ? <li>{t('subRooms.settings.addsDoor')}</li> : null}
            {removesDoor ? <li>{t('subRooms.settings.removesDoor')}</li> : null}
            {voidedRequests > 0 ? (
              <li>{t('subRooms.settings.voidsRequests', { count: voidedRequests })}</li>
            ) : null}
          </ul>
        ) : null}
        {failure === null ? null : knownFailure(failure) ? (
          <Banner
            action={
              failure === 'private_room.conflict' ? (
                <Button
                  onClick={() => {
                    setDraft(draftOf(room));
                    setFailure(null);
                  }}
                  size="compact"
                  tone="quiet"
                >
                  {t('subRooms.settings.startOver')}
                </Button>
              ) : undefined
            }
            title={t(failureMessages[failure])}
            tone="warning"
          />
        ) : (
          <Banner title={t('subRooms.settings.failed')} tone="danger">
            <Details summary={t('subRooms.create.details')}>
              <code>{failure}</code>
            </Details>
          </Banner>
        )}
        <div className="sub-room-settings__save">
          {saved && revision === null ? (
            <p className="sub-room-settings__saved" role="status">
              <Check aria-hidden="true" />
              {t('subRooms.settings.saved')}
            </p>
          ) : null}
          <Button
            aria-busy={saving}
            disabled={!ready || busy}
            icon={
              saving ? (
                <LoaderCircle aria-hidden="true" className="sub-room-settings__spin" />
              ) : undefined
            }
            type="submit"
          >
            {saving ? t('subRooms.settings.saving') : t('subRooms.settings.save')}
          </Button>
        </div>
      </form>
      {managing === null ? null : <SubRoomManagers {...managing} room={room} subRoomRef={ref} />}
      {/* 归档不要最近登录过（specs/cleanup-without-reauth），先在原处确认一次；子房间归档了还能恢复。 */}
      <footer className="sub-room-settings__archive">
        <Button
          aria-expanded={confirmingArchive}
          disabled={busy}
          icon={<Archive aria-hidden="true" />}
          onClick={() => {
            setConfirmingArchive((open) => !open);
            setArchiveFailure(null);
          }}
          ref={archiveButton}
          size="compact"
          tone="alert"
        >
          {t('subRooms.settings.archive')}
        </Button>
        <span className="sub-room-settings__archive-note">
          {t('subRooms.settings.archiveNote')}
        </span>
        {confirmingArchive ? (
          <ArchiveRoomConfirm
            detail={t('subRooms.settings.archiveDetail')}
            name={room.name}
            onCancel={() => {
              setConfirmingArchive(false);
              archiveButton.current?.focus();
            }}
            onConfirm={() => {
              void archive();
            }}
            pending={archiving}
          />
        ) : null}
        {archiveFailure === null ? null : (
          <p className="sub-room-settings__failure" role="alert">
            {knownFailure(archiveFailure)
              ? t(failureMessages[archiveFailure])
              : t('subRooms.failure.other')}
            {knownFailure(archiveFailure) ? null : <code>{archiveFailure}</code>}
          </p>
        )}
      </footer>
    </div>
  );
}
