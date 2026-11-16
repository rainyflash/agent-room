import { Banner, Button, Segmented, Spinner } from '@agent-room/ui-system';
import { useMutation, useQueryClient } from '@tanstack/react-query';
import { ArrowDown, ArrowUp, Pencil, Plus, Trash2 } from 'lucide-react';
import { useEffect, useId, useMemo, useRef, useState, type SubmitEvent } from 'react';
import { useTranslation } from 'react-i18next';

import { useAppServices } from '@/app/app-services';
import {
  subRoomPolicyQueryKey,
  subRoomsQueryKey,
  useSubRoomPolicy,
  useSubRooms,
} from '@/features/private-rooms/data/sub-room-queries';
import type { PrivateRoomFailure } from '@/features/private-rooms/domain/private-room';
import type {
  SubRoomCategory,
  SubRoomPolicy,
  SubRoomPolicySettings,
} from '@/features/private-rooms/domain/sub-rooms';
import { BrowserUuidV7Factory } from '@/shared/ids/browser-uuid-v7-factory';
import { err, type Result } from '@/shared/result';

import './room-sub-rooms-settings.css';

const CATEGORY_NAME_LIMIT = 64;
const unavailable = { code: 'sub_room.unavailable', retryable: false } as const;

/** 说得清的几种没办成；别的说“没办成”，错误码跟在后面。 */
const failureMessages = {
  'private_room.conflict': 'subRooms.roomSettings.failure.conflict',
  'sub_room.category_admins_only': 'subRooms.roomSettings.failure.category_admins_only',
  'sub_room.category_limit_reached': 'subRooms.roomSettings.failure.category_limit_reached',
  'sub_room.category_missing': 'subRooms.create.failure.category_missing',
  'sub_room.category_name_taken': 'subRooms.roomSettings.failure.category_name_taken',
  'sub_room.owner_only': 'subRooms.roomSettings.ownerOnly',
} as const;

function knownFailure(code: string): code is keyof typeof failureMessages {
  return Object.hasOwn(failureMessages, code);
}

function Failure({ code }: { readonly code: string | null }) {
  const { t } = useTranslation();
  if (code === null) return null;
  return (
    <p className="room-sub-rooms__failure" role="alert">
      {knownFailure(code) ? t(failureMessages[code]) : t('subRooms.failure.other')}
      {knownFailure(code) ? null : <code>{code}</code>}
    </p>
  );
}

export type RoomSubRoomsSettingsProps = {
  /** 整间房（父房间）的号。 */
  readonly parentId: string;
};

/**
 * 房间设置里的“子房间”一节（画布第 9 张）：谁能建、Agent 能不能建、30 天没人说话自动归档，只有房主
 * 改得动；下面是分类，房主和管理员建、改名、排先后、删。改了马上存，不另有“保存”。
 */
export function RoomSubRoomsSettings({ parentId }: RoomSubRoomsSettingsProps) {
  const { t } = useTranslation();
  const { subRooms } = useAppServices();
  const policy = useSubRoomPolicy(subRooms, parentId);
  const list = useSubRooms(subRooms, parentId);
  if (policy.isPending || list.isPending) {
    return (
      <p className="room-sub-rooms__state" role="status">
        <Spinner />
        {t('subRooms.roomSettings.loading')}
      </p>
    );
  }
  if (policy.data?.ok !== true || list.data?.ok !== true) {
    return (
      <Banner
        action={
          <Button
            onClick={() => {
              void policy.refetch();
              void list.refetch();
            }}
            size="compact"
            tone="quiet"
          >
            {t('subRooms.roomSettings.retry')}
          </Button>
        }
        tone="warning"
      >
        {t('subRooms.roomSettings.failed')}
      </Banner>
    );
  }
  const counts = new Map<string, number>();
  for (const room of list.data.value.subRooms) {
    const key = room.categoryId ?? '';
    counts.set(key, (counts.get(key) ?? 0) + 1);
  }
  return (
    <div className="room-sub-rooms">
      <p className="room-sub-rooms__intro">{t('subRooms.roomSettings.intro')}</p>
      <PolicySettings parentId={parentId} policy={policy.data.value} />
      <Categories categories={list.data.value.categories} counts={counts} parentId={parentId} />
    </div>
  );
}

function PolicySettings({
  parentId,
  policy,
}: {
  readonly parentId: string;
  readonly policy: SubRoomPolicy;
}) {
  const { t } = useTranslation();
  const { subRooms } = useAppServices();
  const queryClient = useQueryClient();
  const creatorsLabel = useId();
  const change = useMutation({
    mutationFn: async (settings: SubRoomPolicySettings) =>
      subRooms === undefined ? err(unavailable) : await subRooms.changePolicy(parentId, settings),
    onSuccess: async (result) => {
      if (!result.ok) return;
      queryClient.setQueryData(subRoomPolicyQueryKey(parentId), result);
      // 谁能建变了，名单上的“你此刻能不能建”跟着变。
      await queryClient.invalidateQueries({ queryKey: subRoomsQueryKey });
    },
  });
  const pending = change.isPending ? change.variables : undefined;
  // 正在存的那一份先照它显示，存不成再回到服务器说的。
  const shown: SubRoomPolicySettings = pending ?? policy;
  const set = (next: Partial<SubRoomPolicySettings>) => {
    if (!policy.canChange || change.isPending) return;
    change.mutate({
      creators: shown.creators,
      agentsMayCreate: shown.agentsMayCreate,
      autoArchive: shown.autoArchive,
      ...next,
    });
  };
  const failure = change.data?.ok === false ? change.data.error.code : null;
  return (
    <section aria-label={t('subRooms.roomSettings.policy')} className="room-sub-rooms__policy">
      <div className="room-sub-rooms__row">
        <span className="room-sub-rooms__label" id={creatorsLabel}>
          {t('subRooms.roomSettings.creators')}
        </span>
        {policy.canChange ? (
          <Segmented
            className="room-sub-rooms__creators"
            label={t('subRooms.roomSettings.creators')}
            onChange={(creators) => {
              set({ creators });
            }}
            options={[
              { label: t('subRooms.roomSettings.creatorsMembers'), value: 'members' },
              { label: t('subRooms.roomSettings.creatorsAdmins'), value: 'admins' },
            ]}
            value={shown.creators}
          />
        ) : (
          <span aria-labelledby={creatorsLabel} className="room-sub-rooms__value">
            {t(
              shown.creators === 'members'
                ? 'subRooms.roomSettings.creatorsMembers'
                : 'subRooms.roomSettings.creatorsAdmins',
            )}
          </span>
        )}
      </div>
      <Toggle
        checked={shown.agentsMayCreate}
        detail={t('subRooms.roomSettings.agentsDetail')}
        disabled={!policy.canChange || change.isPending}
        label={t('subRooms.roomSettings.agents')}
        onChange={(agentsMayCreate) => {
          set({ agentsMayCreate });
        }}
      />
      <Toggle
        checked={shown.autoArchive}
        detail={t('subRooms.roomSettings.autoArchiveDetail')}
        disabled={!policy.canChange || change.isPending}
        label={t('subRooms.roomSettings.autoArchive')}
        onChange={(autoArchive) => {
          set({ autoArchive });
        }}
      />
      {policy.canChange ? null : (
        <p className="room-sub-rooms__note">{t('subRooms.roomSettings.ownerOnly')}</p>
      )}
      <Failure code={failure} />
    </section>
  );
}

function Toggle({
  checked,
  detail,
  disabled,
  label,
  onChange,
}: {
  readonly checked: boolean;
  readonly detail: string;
  readonly disabled: boolean;
  readonly label: string;
  readonly onChange: (checked: boolean) => void;
}) {
  return (
    <label className="room-sub-rooms__row room-sub-rooms__toggle">
      <span className="room-sub-rooms__text">
        <span className="room-sub-rooms__label">{label}</span>
        <small>{detail}</small>
      </span>
      <input
        checked={checked}
        disabled={disabled}
        onChange={(event) => {
          onChange(event.target.checked);
        }}
        role="switch"
        type="checkbox"
      />
    </label>
  );
}

type CategoryChange =
  | { readonly kind: 'create'; readonly categoryId: string; readonly name: string }
  | { readonly kind: 'rename'; readonly categoryId: string; readonly name: string }
  | { readonly kind: 'move'; readonly categoryId: string; readonly position: number }
  | { readonly kind: 'delete'; readonly categoryId: string };

type CategoryResult = Result<readonly SubRoomCategory[], PrivateRoomFailure>;

function Categories({
  categories,
  counts,
  parentId,
}: {
  readonly categories: readonly SubRoomCategory[];
  readonly counts: ReadonlyMap<string, number>;
  readonly parentId: string;
}) {
  const { t } = useTranslation();
  const { subRooms } = useAppServices();
  const queryClient = useQueryClient();
  const headingId = useId();
  const identifiers = useMemo(() => new BrowserUuidV7Factory(), []);
  // 建的时候先定好分类号：结果不明时再点是同一个请求，服务器不多建。
  const [draft, setDraft] = useState<{ readonly categoryId: string; readonly name: string } | null>(
    null,
  );
  const [renaming, setRenaming] = useState<{
    readonly categoryId: string;
    readonly name: string;
  } | null>(null);
  const [deleting, setDeleting] = useState<string | null>(null);
  const addButton = useRef<HTMLButtonElement>(null);
  const change = useMutation({
    mutationFn: async (next: CategoryChange): Promise<CategoryResult> => {
      if (subRooms === undefined) return err(unavailable);
      switch (next.kind) {
        case 'create':
          return await subRooms.createCategory(parentId, {
            categoryId: next.categoryId,
            name: next.name,
          });
        case 'rename':
          return await subRooms.reviseCategory(parentId, next.categoryId, { name: next.name });
        case 'move':
          return await subRooms.reviseCategory(parentId, next.categoryId, {
            position: next.position,
          });
        case 'delete':
          return await subRooms.deleteCategory(parentId, next.categoryId);
      }
    },
    onSuccess: (result, next) => {
      if (!result.ok) return;
      if (next.kind === 'create') setDraft(null);
      if (next.kind === 'rename') setRenaming(null);
      if (next.kind === 'delete') setDeleting(null);
    },
    onSettled: async () => {
      // 分类挂在子房间的名单上：左栏、浏览子房间、新建的对话框跟着重问。
      await queryClient.invalidateQueries({ queryKey: subRoomsQueryKey });
    },
  });
  const busy = change.isPending;
  const failure = change.data?.ok === false ? change.data.error.code : null;
  const ordered = [...categories].sort((left, right) => left.position - right.position);
  const uncategorized = counts.get('') ?? 0;
  const submitDraft = (event: SubmitEvent<HTMLFormElement>) => {
    event.preventDefault();
    const name = draft?.name.trim() ?? '';
    if (draft === null || name === '' || busy) return;
    change.mutate({ kind: 'create', categoryId: draft.categoryId, name });
  };
  return (
    <section aria-labelledby={headingId} className="room-sub-rooms__categories">
      <div className="room-sub-rooms__head">
        <h3 id={headingId}>
          {t('subRooms.roomSettings.categories')}{' '}
          <span className="room-sub-rooms__count">{ordered.length}</span>
        </h3>
        {draft === null ? (
          <Button
            disabled={busy}
            icon={<Plus aria-hidden="true" />}
            onClick={() => {
              change.reset();
              setDraft({ categoryId: identifiers.next(), name: '' });
            }}
            ref={addButton}
            size="compact"
            tone="quiet"
          >
            {t('subRooms.roomSettings.newCategory')}
          </Button>
        ) : null}
      </div>
      {draft === null ? null : (
        <NameForm
          busy={busy}
          label={t('subRooms.roomSettings.newCategoryName')}
          name={draft.name}
          onCancel={() => {
            setDraft(null);
            change.reset();
            addButton.current?.focus();
          }}
          onChange={(name) => {
            // 换了名字就是另一个请求：换一个分类号，免得撞上“同一个号换了名字”。
            setDraft({ categoryId: identifiers.next(), name });
          }}
          onSubmit={submitDraft}
          submitLabel={t('subRooms.roomSettings.create')}
        />
      )}
      <ol className="room-sub-rooms__list">
        {ordered.map((category, index) => {
          const count = counts.get(category.categoryId) ?? 0;
          const editing = renaming?.categoryId === category.categoryId;
          return (
            <li key={category.categoryId}>
              {editing ? (
                <NameForm
                  busy={busy}
                  label={t('subRooms.roomSettings.renameLabel', { name: category.name })}
                  name={renaming.name}
                  onCancel={() => {
                    setRenaming(null);
                    change.reset();
                  }}
                  onChange={(name) => {
                    setRenaming({ categoryId: category.categoryId, name });
                  }}
                  onSubmit={(event) => {
                    event.preventDefault();
                    const name = renaming.name.trim();
                    if (name === '' || busy) return;
                    if (name === category.name) {
                      setRenaming(null);
                      return;
                    }
                    change.mutate({ kind: 'rename', categoryId: category.categoryId, name });
                  }}
                  submitLabel={t('subRooms.roomSettings.rename')}
                />
              ) : (
                <div className="room-sub-rooms__category">
                  <span className="room-sub-rooms__name">
                    <strong>{category.name}</strong>
                    <small>{t('subRooms.roomSettings.roomCount', { count })}</small>
                  </span>
                  <span className="room-sub-rooms__actions">
                    <Button
                      aria-label={t('subRooms.roomSettings.moveUp', { name: category.name })}
                      disabled={busy || index === 0}
                      icon={<ArrowUp aria-hidden="true" />}
                      onClick={() => {
                        change.mutate({
                          kind: 'move',
                          categoryId: category.categoryId,
                          position: index - 1,
                        });
                      }}
                      size="compact"
                      tone="quiet"
                    />
                    <Button
                      aria-label={t('subRooms.roomSettings.moveDown', { name: category.name })}
                      disabled={busy || index === ordered.length - 1}
                      icon={<ArrowDown aria-hidden="true" />}
                      onClick={() => {
                        change.mutate({
                          kind: 'move',
                          categoryId: category.categoryId,
                          position: index + 1,
                        });
                      }}
                      size="compact"
                      tone="quiet"
                    />
                    <Button
                      aria-label={t('subRooms.roomSettings.renameLabel', { name: category.name })}
                      disabled={busy}
                      icon={<Pencil aria-hidden="true" />}
                      onClick={() => {
                        change.reset();
                        setDeleting(null);
                        setRenaming({ categoryId: category.categoryId, name: category.name });
                      }}
                      size="compact"
                      tone="quiet"
                    />
                    <Button
                      aria-expanded={count > 0 ? deleting === category.categoryId : undefined}
                      aria-label={t('subRooms.roomSettings.deleteLabel', { name: category.name })}
                      disabled={busy}
                      icon={<Trash2 aria-hidden="true" />}
                      onClick={() => {
                        change.reset();
                        // 里面没有子房间的直接删；有的先说一句它们回到“未分类”。
                        if (count === 0)
                          change.mutate({ kind: 'delete', categoryId: category.categoryId });
                        else
                          setDeleting((open) =>
                            open === category.categoryId ? null : category.categoryId,
                          );
                      }}
                      size="compact"
                      tone="quiet"
                    />
                  </span>
                </div>
              )}
              {deleting === category.categoryId ? (
                <div
                  aria-label={t('subRooms.roomSettings.deleteTitle', { name: category.name })}
                  className="room-sub-rooms__confirm"
                  role="group"
                >
                  <p>{t('subRooms.roomSettings.deleteDetail', { count })}</p>
                  <span className="room-sub-rooms__confirm-actions">
                    <Button
                      disabled={busy}
                      onClick={() => {
                        setDeleting(null);
                      }}
                      size="compact"
                      tone="quiet"
                    >
                      {t('subRooms.roomSettings.cancel')}
                    </Button>
                    <Button
                      aria-busy={busy}
                      disabled={busy}
                      onClick={() => {
                        change.mutate({ kind: 'delete', categoryId: category.categoryId });
                      }}
                      size="compact"
                    >
                      {t('subRooms.roomSettings.delete')}
                    </Button>
                  </span>
                </div>
              ) : null}
            </li>
          );
        })}
        <li className="room-sub-rooms__uncategorized">
          <div className="room-sub-rooms__category">
            <span className="room-sub-rooms__name">
              <strong>{t('subRooms.browse.uncategorized')}</strong>
              <small>{t('subRooms.roomSettings.roomCount', { count: uncategorized })}</small>
            </span>
            <small className="room-sub-rooms__hint">
              {t('subRooms.roomSettings.uncategorizedHint')}
            </small>
          </div>
        </li>
      </ol>
      <Failure code={failure} />
      <p className="room-sub-rooms__note">{t('subRooms.roomSettings.categoriesNote')}</p>
    </section>
  );
}

function NameForm({
  busy,
  label,
  name,
  onCancel,
  onChange,
  onSubmit,
  submitLabel,
}: {
  readonly busy: boolean;
  readonly label: string;
  readonly name: string;
  readonly onCancel: () => void;
  readonly onChange: (name: string) => void;
  readonly onSubmit: (event: SubmitEvent<HTMLFormElement>) => void;
  readonly submitLabel: string;
}) {
  const { t } = useTranslation();
  const input = useRef<HTMLInputElement>(null);
  useEffect(() => {
    input.current?.focus();
  }, []);
  return (
    <form className="room-sub-rooms__name-form" onSubmit={onSubmit}>
      <input
        aria-label={label}
        disabled={busy}
        maxLength={CATEGORY_NAME_LIMIT}
        onChange={(event) => {
          onChange(event.target.value);
        }}
        onKeyDown={(event) => {
          if (event.key === 'Escape') {
            event.stopPropagation();
            event.preventDefault();
            onCancel();
          }
        }}
        ref={input}
        value={name}
      />
      <Button disabled={busy} onClick={onCancel} size="compact" tone="quiet">
        {t('subRooms.roomSettings.cancel')}
      </Button>
      <Button aria-busy={busy} disabled={busy || name.trim() === ''} size="compact" type="submit">
        {submitLabel}
      </Button>
    </form>
  );
}
