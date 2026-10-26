import { Banner, Button, CopyBlock, Details, Spinner } from '@agent-room/ui-system';
import { useMutation, useQueryClient } from '@tanstack/react-query';
import { KeyRound } from 'lucide-react';
import { useState } from 'react';
import { useTranslation } from 'react-i18next';

import {
  privateRoomAgentAccessQueryKey,
  privateRoomJoinCodeQueryKey,
  usePrivateRoomAgentAccess,
  usePrivateRoomJoinCode,
  usePrivateRoomList,
} from '@/features/private-rooms/data/private-room-queries';
import {
  allows,
  memberFor,
  type JoinCode,
  type PrivateRoomAgentAccess,
  type PrivateRoomFailure,
  type PrivateRoomGateway,
} from '@/features/private-rooms/domain/private-room';
import { PrivateRoomAgentKnocks } from '@/features/private-rooms/ui/private-room-agent-knocks';
import { PrivateRoomFailureNotice } from '@/features/private-rooms/ui/private-room-failure-notice';
import { ok, type Result } from '@/shared/result';

/** 服务器拒绝非管理者查看或生成口令时的错误码。 */
const forbiddenCodes = new Set(['join_code.forbidden', 'private_room.forbidden']);
/** 查口令时这两种回答不算出错：没有口令，或者有口令但服务器给不出来。 */
const expectedRevealFailures = new Set(['join_code.not_found', 'join_code.not_revealable']);

type InviteProps = {
  readonly catalogId: string;
  readonly guide: string;
  readonly roomName: string;
  readonly rooms: PrivateRoomGateway;
  readonly onCopied?: (() => void) | undefined;
  /** 看的人是谁。知道了就先按房间列表判断是不是管理者，不是就不去问口令状态。 */
  readonly principalId?: string | null | undefined;
};

/** 管理者这边的口令：看得到就放进话里；没有，或者有却看不到，就给一个生成的按钮。 */
type CodeState =
  | { readonly kind: 'shown'; readonly code: JoinCode }
  | { readonly kind: 'none' }
  | { readonly kind: 'hidden'; readonly failure: PrivateRoomFailure | null };

/**
 * 在“接入 Agent”里请网络 Agent 进私人房间（`specs/network-agents/code-first.md`）。管理者给 Agent 的话里
 * 带着口令，Agent 凭它直接进来；同一段话里还有房间号，口令传不进去的 Agent（有的应用不让把口令交给工具）
 * 拿它敲门，管理者在下面放它进来。还没有口令，或者口令看不到（能再看之前生成的），一个按钮生成并复制。
 * 不是管理者的成员看不到口令，话里只有房间号。房间号不是秘密：房间里每个人的地址栏里都有。
 */
export function PrivateRoomNetworkInvite(props: InviteProps) {
  const { t } = useTranslation();
  const { catalogId, principalId = null, rooms } = props;
  const manages = useManagesRoom(rooms, catalogId, principalId);
  // 口令状态只有管理者查得到，顺便知道这段话能不能带口令。房间列表说了不是管理者就不问：
  // 问了服务器也只回 403，还记成一条错（agent-room-ops#12）。
  const access = usePrivateRoomAgentAccess(rooms, catalogId, manages === true || manages === null);
  const createdAt =
    access.data?.ok === true ? (access.data.value.joinCode?.createdAtUnixMs ?? null) : null;
  const revealed = usePrivateRoomJoinCode(rooms, catalogId, createdAt);
  if (manages === false) return <MemberInvite {...props} failure={null} />;
  if (manages === 'pending' || access.isPending || (createdAt !== null && revealed.isPending)) {
    return (
      <p className="agent-invite__waiting">
        <Spinner />
        {t('agentInvite.network.checking')}
      </p>
    );
  }
  if (access.data?.ok !== true) {
    const failure =
      access.data === undefined || forbiddenCodes.has(access.data.error.code)
        ? null
        : access.data.error;
    return <MemberInvite {...props} failure={failure} />;
  }
  return <ManagerInvite {...props} code={codeState(createdAt, revealed.data)} />;
}

/**
 * 按房间列表看这个人能不能管这个房间（房主，或者有“管理”权限的已加入成员，和服务器、房间设置一样）。
 * 还在读是 `'pending'`；不知道是谁、列表没读到或者里面没有这个房间是 `null`，这时照旧去问服务器。
 */
function useManagesRoom(
  rooms: PrivateRoomGateway,
  catalogId: string,
  principalId: string | null,
): boolean | 'pending' | null {
  const list = usePrivateRoomList(rooms, principalId ?? undefined, principalId !== null);
  if (principalId === null) return null;
  if (list.isPending) return 'pending';
  if (list.data?.ok !== true) return null;
  const room = list.data.value.find((entry) => entry.catalogId === catalogId);
  if (room === undefined) return null;
  return room.ownerPrincipalId === principalId || allows(memberFor(room, principalId), 'manage');
}

function codeState(
  createdAt: number | null,
  revealed: Result<JoinCode, PrivateRoomFailure> | undefined,
): CodeState {
  if (createdAt === null || revealed === undefined) return { kind: 'none' };
  if (revealed.ok) return { kind: 'shown', code: revealed.value };
  // 别处刚停用了口令：当作没有。
  if (revealed.error.code === 'join_code.not_found') return { kind: 'none' };
  return {
    kind: 'hidden',
    failure: expectedRevealFailures.has(revealed.error.code) ? null : revealed.error,
  };
}

/** 不是管理者：看不到口令，话里只有房间号，Agent 敲门后等房间的管理者放行。 */
function MemberInvite({
  catalogId,
  guide,
  roomName,
  onCopied,
  failure,
}: InviteProps & { readonly failure: PrivateRoomFailure | null }) {
  const { t } = useTranslation();
  return (
    <div className="agent-invite__private-code">
      <p className="agent-invite__note">
        {t('agentInvite.network.privateRoomMember', { room: roomName })}
      </p>
      <CopyBlock
        copiedLabel={t('agentInvite.message.copied')}
        copyLabel={t('agentInvite.message.copy')}
        failedLabel={t('agentInvite.message.failed')}
        onCopied={onCopied}
        text={t('agentInvite.network.knock.messageMember', {
          guide,
          room: roomName,
          roomNumber: catalogId,
        })}
        textLabel={t('agentInvite.message.label')}
      />
      <p className="agent-invite__note">{t('agentInvite.network.knock.memberNote')}</p>
      {failure === null ? null : <PrivateRoomFailureNotice failure={failure} />}
      <p className="agent-invite__note">{t('agentInvite.network.private.relay')}</p>
    </div>
  );
}

/**
 * 管理者：看得到口令就把它放进话里，打开就能复制；没有口令或者看不到，一个按钮生成、把整段话复制好。
 * 不想用口令的，折叠里有只带房间号的那段话。在敲门的 Agent 列在下面，放行照旧。
 */
function ManagerInvite({
  catalogId,
  guide,
  roomName,
  rooms,
  onCopied,
  code,
}: InviteProps & { readonly code: CodeState }) {
  const { t } = useTranslation();
  const queryClient = useQueryClient();
  const [autoCopied, setAutoCopied] = useState(false);
  const messageFor = (value: string) =>
    t('agentInvite.network.code.message', {
      code: value,
      guide,
      room: roomName,
      roomNumber: catalogId,
    });

  const mutation = useMutation({
    mutationFn: async () => await rooms.generateJoinCode(catalogId),
    onSuccess: async (result) => {
      if (!result.ok) return;
      const generated = result.value;
      // 先记下新口令，再改口令状态：话里马上换成新口令，不用再去问一次。房间设置里的口令一块也读这两份缓存。
      queryClient.setQueryData<Result<JoinCode, PrivateRoomFailure>>(
        privateRoomJoinCodeQueryKey(catalogId, generated.createdAtUnixMs),
        ok(generated),
      );
      const accessKey = privateRoomAgentAccessQueryKey(catalogId);
      queryClient.setQueryData<Result<PrivateRoomAgentAccess, PrivateRoomFailure>>(
        accessKey,
        (current) =>
          current?.ok === true
            ? ok({ ...current.value, joinCode: { createdAtUnixMs: generated.createdAtUnixMs } })
            : current,
      );
      // 一次点击就复制好。有的浏览器等完服务器就不认那次点击了，这时话旁边照样有复制按钮。
      try {
        await navigator.clipboard.writeText(messageFor(generated.code));
        setAutoCopied(true);
        onCopied?.();
      } catch {
        // 不算错。
      }
      await queryClient.invalidateQueries({ queryKey: accessKey });
    },
  });

  const failure = mutation.data?.ok === false ? mutation.data.error : null;
  const forbidden = failure !== null && forbiddenCodes.has(failure.code);

  return (
    <div className="agent-invite__private-code">
      <p className="agent-invite__note">
        {t('agentInvite.network.privateRoom', { room: roomName })}
      </p>
      {code.kind === 'shown' ? (
        <>
          {autoCopied ? <Banner tone="success">{t('agentInvite.message.copied')}</Banner> : null}
          <CopyBlock
            copiedLabel={t('agentInvite.message.copied')}
            copyLabel={t('agentInvite.message.copy')}
            failedLabel={t('agentInvite.message.failed')}
            onCopied={onCopied}
            text={messageFor(code.code.code)}
            textLabel={t('agentInvite.message.label')}
          />
          <p className="agent-invite__note">{t('agentInvite.network.code.detail')}</p>
        </>
      ) : (
        <>
          {forbidden ? (
            <Banner role={null} tone="info">
              {t('agentInvite.network.private.forbidden')}
            </Banner>
          ) : (
            <Button
              disabled={mutation.isPending}
              icon={mutation.isPending ? <Spinner /> : <KeyRound aria-hidden="true" />}
              onClick={() => {
                mutation.mutate();
              }}
              size="large"
              tone="primary"
            >
              {t(
                code.kind === 'hidden'
                  ? 'agentInvite.network.private.replace'
                  : 'agentInvite.network.private.create',
              )}
            </Button>
          )}
          {code.kind === 'hidden' ? (
            <p className="agent-invite__note">{t('agentInvite.network.private.replaceNote')}</p>
          ) : null}
          {code.kind === 'hidden' && code.failure !== null ? (
            <PrivateRoomFailureNotice failure={code.failure} />
          ) : null}
          <Details
            className="agent-invite__knock-only"
            summary={t('agentInvite.network.knock.summary')}
          >
            <CopyBlock
              copiedLabel={t('agentInvite.message.copied')}
              copyLabel={t('agentInvite.message.copy')}
              failedLabel={t('agentInvite.message.failed')}
              onCopied={onCopied}
              text={t('agentInvite.network.knock.message', {
                guide,
                room: roomName,
                roomNumber: catalogId,
              })}
              textLabel={t('agentInvite.message.label')}
              tone="ghost"
            />
          </Details>
        </>
      )}
      {failure === null || forbidden ? null : <PrivateRoomFailureNotice failure={failure} />}
      <PrivateRoomAgentKnocks
        catalogId={catalogId}
        emptySpinner={false}
        emptyText={t('agentInvite.network.knock.waiting')}
        rooms={rooms}
      />
      <p className="agent-invite__note">{t('agentInvite.network.private.relay')}</p>
    </div>
  );
}
