import { Button, Spinner } from '@agent-room/ui-system';
import { useMutation, useQuery } from '@tanstack/react-query';
import { DoorOpen, RotateCw, TriangleAlert, UserPlus } from 'lucide-react';
import { useEffect, useState } from 'react';
import { useTranslation } from 'react-i18next';

import { useAppServices } from '@/app/app-services';
import {
  forgetInviteSecret,
  readInviteSecret,
} from '@/features/private-rooms/adapters/browser-invite-secret';
import type { PrivateRoomFailure } from '@/features/private-rooms/domain/private-room';
import type { PrivateRoomInvitePreview } from '@/features/private-rooms/domain/private-room-invite-link';
import { useRoomCommands } from '@/features/room-directory/ui/use-rooms';
import { sessionStateName } from '@/features/session/ui/connection-model';
import { useSession } from '@/features/session/ui/session-provider';
import { formatDateTime } from '@/shared/i18n/formatters';
import { err, type Result } from '@/shared/result';
import { EntryCard, EntryShell } from '@/shared/ui/entry-shell';

export type JoinInvitePageProps = {
  /** 没登录：先去登录，回来以后接着打开这条邀请（秘密收在这个标签页里）。 */
  readonly onConnectionRequired: () => void;
};

/** 这几种回答说明链接本身没用了，秘密也不用再留着。 */
const finalFailures = new Set([
  'invite_link.not_found',
  'invite_link.banned',
  'invite_link.invalid',
]);

const failureCopies = {
  expired: {
    title: 'privateRooms.join.expired.title',
    detail: 'privateRooms.join.expired.detail',
    retry: false,
  },
  banned: {
    title: 'privateRooms.join.banned.title',
    detail: 'privateRooms.join.banned.detail',
    retry: false,
  },
  missing: {
    title: 'privateRooms.join.missing.title',
    detail: 'privateRooms.join.missing.detail',
    retry: false,
  },
  busy: {
    title: 'privateRooms.join.busy.title',
    detail: 'privateRooms.join.busy.detail',
    retry: true,
  },
  failed: {
    title: 'privateRooms.join.failed.title',
    detail: 'privateRooms.join.failed.detail',
    retry: true,
  },
} as const;

function failureCopy(code: string): (typeof failureCopies)[keyof typeof failureCopies] {
  switch (code) {
    case 'invite_link.not_found':
      return failureCopies.expired;
    case 'invite_link.banned':
      return failureCopies.banned;
    case 'invite_link.invalid':
      return failureCopies.missing;
    case 'invite_link.rate_limited':
      return failureCopies.busy;
    default:
      return failureCopies.failed;
  }
}

/**
 * 打开别人发来的邀请链接（`specs/room-people/design.md`）：先看是哪个房间、谁邀请的，点“加入”
 * 以链接创建人的名义受邀，再照接受邀请那一套进房间，不用再点一次。已经在房间里的直接打开。
 */
export function JoinInvitePage({ onConnectionRequired }: JoinInvitePageProps) {
  const { privateRoomInvites, privateRooms } = useAppServices();
  const { snapshot } = useSession();
  const commands = useRoomCommands();
  const sessionState = sessionStateName(snapshot.value);
  const ready = sessionState === 'ready' && snapshot.context.principal !== null;
  const [secret] = useState(() => readInviteSecret());

  useEffect(() => {
    if (sessionState === 'unauthenticated' && snapshot.context.principal === null) {
      onConnectionRequired();
    }
  }, [onConnectionRequired, sessionState, snapshot.context.principal]);

  const preview = useQuery({
    enabled: ready && secret !== null && privateRoomInvites !== undefined,
    // 键里不放秘密：查询缓存会出现在调试工具里。一个标签页同时只开一条邀请。
    queryKey: ['private-room-invite-preview'] as const,
    queryFn: async () =>
      privateRoomInvites === undefined || secret === null
        ? err<PrivateRoomFailure>({ code: 'invite_link.invalid', retryable: false })
        : await privateRoomInvites.previewInvite(secret),
    gcTime: 0,
    networkMode: 'always',
    retry: false,
    staleTime: Infinity,
  });
  const join = useMutation({
    mutationFn: async (): Promise<Result<unknown, PrivateRoomFailure>> => {
      if (privateRoomInvites === undefined || secret === null) {
        return err({ code: 'invite_link.invalid', retryable: false });
      }
      const redeemed = await privateRoomInvites.redeemInvite(secret);
      if (!redeemed.ok) return redeemed;
      const room = await privateRooms.inspect(redeemed.value.catalogId);
      if (!room.ok) return room;
      // 受邀还没答复的，照接受邀请那一套：Matrix 上加入，再告诉服务器；成功后直接跳进房间。
      return await commands.enter(room.value, redeemed.value.membership !== 'joined');
    },
    onSuccess: (result) => {
      if (result.ok || finalFailures.has(result.error.code)) forgetInviteSecret();
    },
  });

  // 打开时就知道链接没用了（过期、停用、被封禁）：秘密也不用再留着。
  const previewFailure = preview.data?.ok === false ? preview.data.error.code : null;
  useEffect(() => {
    if (previewFailure !== null && finalFailures.has(previewFailure)) forgetInviteSecret();
  }, [previewFailure]);

  const failure =
    secret === null
      ? ({ code: 'invite_link.invalid', retryable: false } satisfies PrivateRoomFailure)
      : join.data?.ok === false
        ? join.data.error
        : preview.data?.ok === false
          ? preview.data.error
          : null;
  const state: JoinInviteState =
    failure !== null
      ? { kind: 'failure', failure }
      : !ready || preview.data?.ok !== true
        ? { kind: 'loading' }
        : {
            kind: 'preview',
            joining: join.isPending || join.data?.ok === true,
            preview: preview.data.value,
          };
  return (
    <JoinInviteView
      onJoin={() => {
        join.mutate();
      }}
      onRetry={() => {
        join.reset();
        void preview.refetch();
      }}
      state={state}
    />
  );
}

export type JoinInviteState =
  | { readonly kind: 'loading' }
  | { readonly kind: 'failure'; readonly failure: PrivateRoomFailure }
  | {
      readonly kind: 'preview';
      readonly joining: boolean;
      readonly preview: PrivateRoomInvitePreview;
    };

/** 打开邀请链接的那一页长什么样：浏览器验收的夹具直接用它。 */
export function JoinInviteView({
  onJoin,
  onRetry,
  state,
}: {
  readonly onJoin: () => void;
  readonly onRetry: () => void;
  readonly state: JoinInviteState;
}) {
  const { t } = useTranslation();
  if (state.kind === 'failure') return <JoinFailure failure={state.failure} onRetry={onRetry} />;
  if (state.kind === 'loading') {
    return (
      <EntryShell>
        <EntryCard icon={<Spinner />} title={t('privateRooms.join.loading')} />
      </EntryShell>
    );
  }
  return <JoinPreview joining={state.joining} onJoin={onJoin} preview={state.preview} />;
}

function JoinPreview({
  joining,
  onJoin,
  preview,
}: {
  readonly joining: boolean;
  readonly onJoin: () => void;
  readonly preview: PrivateRoomInvitePreview;
}) {
  const { t, i18n } = useTranslation();
  const inviter = preview.inviterDisplayName.trim();
  const capabilities = preview.permissions.capabilities;
  const already = preview.membership === 'joined';
  const title = already
    ? t('privateRooms.join.already', { room: preview.roomName })
    : inviter.length === 0
      ? t('privateRooms.join.titleUnnamed', { room: preview.roomName })
      : t('privateRooms.join.title', { name: inviter, room: preview.roomName });
  const detail = already
    ? undefined
    : `${t(
        capabilities.includes('automate')
          ? 'privateRooms.join.permissions.automate'
          : capabilities.includes('speak')
            ? 'privateRooms.join.permissions.speak'
            : 'privateRooms.join.permissions.view',
      )} ${t('privateRooms.join.expires', {
        date: formatDateTime(preview.expiresAtUnixMs, i18n.resolvedLanguage),
      })}`;
  return (
    <EntryShell>
      <EntryCard
        actions={
          <>
            <Button
              disabled={joining}
              icon={already ? <DoorOpen aria-hidden="true" /> : <UserPlus aria-hidden="true" />}
              onClick={onJoin}
              size="large"
              tone="primary"
            >
              {joining
                ? t('privateRooms.join.joining')
                : t(already ? 'privateRooms.join.open' : 'privateRooms.join.accept')}
            </Button>
            <a className="ar-button ar-button--large ar-button--ghost" href="/rooms">
              {t('entry.backToRooms')}
            </a>
          </>
        }
        {...(detail === undefined ? {} : { detail })}
        icon={<UserPlus />}
        title={title}
      />
    </EntryShell>
  );
}

function JoinFailure({
  failure,
  onRetry,
}: {
  readonly failure: PrivateRoomFailure;
  readonly onRetry: () => void;
}) {
  const { t } = useTranslation();
  const copy = failureCopy(failure.code);
  return (
    <EntryShell>
      <EntryCard
        actions={
          <>
            {copy.retry ? (
              <Button
                icon={<RotateCw aria-hidden="true" />}
                onClick={onRetry}
                size="large"
                tone="primary"
              >
                {t('privateRooms.join.retry')}
              </Button>
            ) : null}
            <a
              className={`ar-button ar-button--large ${copy.retry ? 'ar-button--ghost' : 'ar-button--primary'}`}
              href="/rooms"
            >
              {t('entry.backToRooms')}
            </a>
          </>
        }
        detail={t(copy.detail)}
        details={
          <dl>
            <div>
              <dt>{t('entry.errorCode')}</dt>
              <dd>
                <code>{failure.code}</code>
              </dd>
            </div>
          </dl>
        }
        icon={<TriangleAlert />}
        title={t(copy.title)}
        tone="alert"
      />
    </EntryShell>
  );
}
