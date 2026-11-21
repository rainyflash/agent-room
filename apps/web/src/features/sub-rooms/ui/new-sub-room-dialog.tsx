import { Banner, Button, Details, Dialog, Field, Spinner } from '@agent-room/ui-system';
import { useQueryClient } from '@tanstack/react-query';
import { useId, useMemo, useRef, useState } from 'react';
import { useTranslation } from 'react-i18next';

import { useAppServices } from '@/app/app-services';
import type { LobbyAgent } from '@/features/lobby/domain/lobby';
import { subRoomsQueryKey } from '@/features/private-rooms/data/sub-room-queries';
import {
  subRoomAccessValues,
  type CreateSubRoomInput,
  type SubRoomAccess,
  type SubRoomList,
} from '@/features/private-rooms/domain/sub-rooms';
import { useRoomCommands } from '@/features/room-directory/ui/use-rooms';
import { useOwnedAgents } from '@/features/workspace/data/agent-directory-query';
import { bringWhenReady } from '@/features/workspace/domain/agent-rooms';
import { BrowserUuidV7Factory } from '@/shared/ids/browser-uuid-v7-factory';

import '@/features/private-rooms/ui/private-room-public-settings.css';
import './new-sub-room-dialog.css';

const NAME_LIMIT = 128;
const PURPOSE_LIMIT = 2_048;

/** 说得清的几种没建成；别的说“没建成”，错误码收在“详情”里。 */
const failureMessages = {
  'sub_room.admins_only': 'subRooms.create.failure.admins_only',
  'sub_room.category_missing': 'subRooms.create.failure.category_missing',
  'sub_room.limit_reached': 'subRooms.create.failure.limit_reached',
  'sub_room.name_taken': 'subRooms.create.failure.name_taken',
  'sub_room.rate_limited': 'subRooms.create.failure.rate_limited',
} as const;

/** 服务器说这份填写不行：什么都没建，改了再建是一个新请求（换一个房间号）。 */
const REJECTED_INPUT = new Set<string>([
  'sub_room.invalid_creation_body',
  'sub_room.category_missing',
  'sub_room.name_taken',
]);

function knownFailure(code: string): code is keyof typeof failureMessages {
  return Object.hasOwn(failureMessages, code);
}

export type NewSubRoomDialogProps = {
  /** 这间房下面的名单：父房间和分类从这里来。 */
  readonly list: SubRoomList;
  /** 眼前这间房里的 Agent：你的那几个可以一起带进新的子房间。 */
  readonly roomAgents: readonly LobbyAgent[];
  readonly onClose: () => void;
};

/**
 * 新建子房间（`specs/redesign/sub-rooms.md`“网页”，画布第 7 张）：名字、用途、分类、谁能进（默认“直接进”），
 * 一个按钮建好并进去。可以勾上“把我在这间房里的 Agent 也带进来”（就是“我的 Agent”里的“带进房间”）。
 */
export function NewSubRoomDialog({ list, onClose, roomAgents }: NewSubRoomDialogProps) {
  const { t } = useTranslation();
  const { agentDirectory, subRooms } = useAppServices();
  const commands = useRoomCommands();
  const queries = useQueryClient();
  const formId = useId();
  const identifiers = useMemo(() => new BrowserUuidV7Factory(), []);
  const owned = useOwnedAgents(agentDirectory);
  const mine = useMemo(() => {
    if (owned.data?.ok !== true) return [];
    const ids = new Set(owned.data.value.map((agent) => agent.agentId));
    // 同一个 Agent 的几个连接在名单上是几行，只带一次。
    const found = new Map<string, LobbyAgent>();
    for (const agent of roomAgents) {
      if (ids.has(agent.agentId) && !found.has(agent.agentId)) found.set(agent.agentId, agent);
    }
    return [...found.values()];
  }, [owned.data, roomAgents]);
  const [name, setName] = useState('');
  const [purpose, setPurpose] = useState('');
  const [categoryId, setCategoryId] = useState('');
  const [access, setAccess] = useState<SubRoomAccess>('open');
  const [bring, setBring] = useState(true);
  const [pending, setPending] = useState(false);
  const pendingRef = useRef(false);
  const [failure, setFailure] = useState<string | null>(null);
  // 建好了、没进去（多半是在 Matrix 上加入没成）：再点一次是同一个请求，服务器回同一间，接着进。
  const [created, setCreated] = useState(false);
  // 第一次提交后请求就定下来了：结果不明、进去没成时重试是同一个请求，服务器回同一间。
  const attempt = useRef<{
    readonly catalogId: string;
    readonly input: CreateSubRoomInput;
  } | null>(null);
  const locked = attempt.current !== null;
  const trimmed = name.trim();
  const nameError = trimmed.includes('/') ? t('subRooms.create.nameSlash') : undefined;
  const ready = trimmed.length > 0 && nameError === undefined;
  const parentId = list.parent.catalogId;

  const submit = async (): Promise<void> => {
    if (pendingRef.current || (!locked && !ready) || subRooms === undefined) return;
    pendingRef.current = true;
    if (attempt.current === null) {
      const catalogId = identifiers.next();
      attempt.current = {
        catalogId,
        input: {
          catalogId,
          name: trimmed,
          description: purpose.trim(),
          access,
          ...(categoryId === '' ? {} : { categoryId }),
        },
      };
    }
    const { catalogId, input } = attempt.current;
    setPending(true);
    setFailure(null);
    try {
      const created = await subRooms.create(parentId, input);
      if (!created.ok) {
        if (REJECTED_INPUT.has(created.error.code)) attempt.current = null;
        setFailure(created.error.code);
        return;
      }
      setCreated(true);
      await queries.invalidateQueries({ queryKey: subRoomsQueryKey });
      // 带不进去的不挡着人进：以后在“我的 Agent”里再带。
      if (bring)
        await Promise.all(
          mine.map((agent) =>
            bringWhenReady(
              () => agentDirectory.bringAgentIntoRoom(agent.agentId, catalogId),
              (milliseconds) =>
                new Promise((resolve) => {
                  window.setTimeout(resolve, milliseconds);
                }),
            ),
          ),
        );
      const entered = await commands.enter(created.value, false);
      if (entered.ok) {
        onClose();
        return;
      }
      setFailure(entered.error.code);
    } catch {
      setFailure('sub_room.create_interrupted');
    } finally {
      pendingRef.current = false;
      setPending(false);
    }
  };

  const rejected = failure !== null && REJECTED_INPUT.has(failure);
  return (
    <Dialog
      className="new-sub-room"
      closeLabel={t('subRooms.create.close')}
      description={t('subRooms.create.under', { room: list.parent.name })}
      footer={
        <>
          <span className="new-sub-room__note">{t('subRooms.create.youManage')}</span>
          <Button disabled={pending} onClick={onClose} tone="quiet" type="button">
            {t('subRooms.create.cancel')}
          </Button>
          <Button
            disabled={pending || (!locked && !ready)}
            form={formId}
            icon={pending ? <Spinner /> : undefined}
            type="submit"
          >
            {t(
              pending
                ? 'subRooms.create.creating'
                : failure === null || rejected
                  ? 'subRooms.create.submit'
                  : 'subRooms.create.retry',
            )}
          </Button>
        </>
      }
      onClose={() => {
        if (!pending) onClose();
      }}
      title={t('subRooms.create.title')}
    >
      <form
        className="new-sub-room__form"
        id={formId}
        onSubmit={(event) => {
          event.preventDefault();
          void submit();
        }}
      >
        <Field error={nameError} label={t('subRooms.create.name')}>
          <input
            data-autofocus
            disabled={pending || locked}
            maxLength={NAME_LIMIT}
            onChange={(event) => {
              setName(event.target.value);
              setFailure(null);
            }}
            required
            value={name}
          />
        </Field>
        <Field
          hint={t('subRooms.create.purposeHint')}
          label={
            <>
              {t('subRooms.create.purpose')}{' '}
              <span className="new-sub-room__optional">{t('subRooms.create.optional')}</span>
            </>
          }
        >
          <textarea
            disabled={pending || locked}
            maxLength={PURPOSE_LIMIT}
            onChange={(event) => {
              setPurpose(event.target.value);
              setFailure(null);
            }}
            rows={2}
            value={purpose}
          />
        </Field>
        {list.categories.length === 0 ? null : (
          <Field label={t('subRooms.create.category')}>
            <select
              disabled={pending || locked}
              onChange={(event) => {
                setCategoryId(event.target.value);
                setFailure(null);
              }}
              value={categoryId}
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
        <fieldset
          className="room-listing__choices room-listing__choices--three"
          disabled={pending || locked}
        >
          <legend>{t('subRooms.create.access')}</legend>
          {subRoomAccessValues.map((value) => (
            <label key={value}>
              <input
                checked={access === value}
                name={`new-sub-room-access-${formId}`}
                onChange={() => {
                  setAccess(value);
                  setFailure(null);
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
        {mine.length === 0 ? null : (
          <label className="new-sub-room__bring">
            <input
              checked={bring}
              disabled={pending || locked}
              onChange={(event) => {
                setBring(event.target.checked);
              }}
              type="checkbox"
            />
            <span>
              <span>{t('subRooms.create.bring')}</span>
              <small>
                {mine.map((agent) => agent.displayName).join(t('subRooms.create.listSeparator'))}
              </small>
            </span>
          </label>
        )}
      </form>
      {failure === null ? null : knownFailure(failure) ? (
        <Banner tone="warning" title={t(failureMessages[failure])} />
      ) : (
        <Banner
          tone="danger"
          title={t(created ? 'subRooms.create.enterFailed' : 'subRooms.create.failed')}
        >
          <Details summary={t('subRooms.create.details')}>
            <code>{failure}</code>
          </Details>
        </Banner>
      )}
    </Dialog>
  );
}
