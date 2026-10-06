import { Banner, Button, CopyBlock, Details, Spinner } from '@agent-room/ui-system';
import { useMutation, useQueryClient } from '@tanstack/react-query';
import { KeyRound } from 'lucide-react';
import { useState } from 'react';
import { useTranslation } from 'react-i18next';

import {
  privateRoomAgentAccessQueryKey,
  usePrivateRoomAgentAccess,
} from '@/features/private-rooms/data/private-room-queries';
import type {
  GeneratedJoinCode,
  PrivateRoomAgentAccess,
  PrivateRoomFailure,
  PrivateRoomGateway,
} from '@/features/private-rooms/domain/private-room';
import { PrivateRoomAgentKnocks } from '@/features/private-rooms/ui/private-room-agent-knocks';
import { PrivateRoomFailureNotice } from '@/features/private-rooms/ui/private-room-failure-notice';
import { ok, type Result } from '@/shared/result';

/** 服务器拒绝非管理者查看或生成口令时的错误码。 */
const forbiddenCodes = new Set(['join_code.forbidden', 'private_room.forbidden']);

type InviteProps = {
  readonly catalogId: string;
  readonly guide: string;
  readonly roomName: string;
  readonly rooms: PrivateRoomGateway;
  readonly onCopied?: (() => void) | undefined;
};

/**
 * 在“接入 Agent”里请网络 Agent 进私人房间（`specs/network-agents/knock.md`）：给 Agent 的话里只有
 * 房间号，打开就能复制；Agent 拿它敲门，管理者在下面放它进来。口令收进折叠里，给不用等放行的 Agent。
 * 房间号不是秘密：房间里每个人的地址栏里都有，拿到它也看不到房间里的任何东西。
 */
export function PrivateRoomNetworkInvite({
  catalogId,
  guide,
  roomName,
  rooms,
  onCopied,
}: InviteProps) {
  const { t } = useTranslation();
  // 查口令状态只有管理者查得到：顺便知道这段话该说“等我放行”还是“等管理者放行”。
  const access = usePrivateRoomAgentAccess(rooms, catalogId);
  if (access.isPending) {
    return (
      <p className="agent-invite__waiting">
        <Spinner />
        {t('agentInvite.network.checking')}
      </p>
    );
  }
  const manager = access.data?.ok === true;
  const failure =
    access.data?.ok === false && !forbiddenCodes.has(access.data.error.code)
      ? access.data.error
      : null;
  const message = t(
    manager ? 'agentInvite.network.knock.message' : 'agentInvite.network.knock.messageMember',
    { guide, room: roomName, roomNumber: catalogId },
  );

  return (
    <div className="agent-invite__private-code">
      <p className="agent-invite__note">
        {t(
          manager ? 'agentInvite.network.privateRoom' : 'agentInvite.network.privateRoomMember',
          { room: roomName },
        )}
      </p>
      <CopyBlock
        copiedLabel={t('agentInvite.message.copied')}
        copyLabel={t('agentInvite.message.copy')}
        failedLabel={t('agentInvite.message.failed')}
        onCopied={onCopied}
        text={message}
        textLabel={t('agentInvite.message.label')}
      />
      {manager ? (
        <PrivateRoomAgentKnocks
          catalogId={catalogId}
          emptyText={t('agentInvite.network.knock.waiting')}
          rooms={rooms}
        />
      ) : (
        <p className="agent-invite__note">{t('agentInvite.network.knock.memberNote')}</p>
      )}
      {failure === null ? null : <PrivateRoomFailureNotice failure={failure} />}
      {manager ? (
        <Details className="agent-invite__code" summary={t('agentInvite.network.code.summary')}>
          <JoinCodeInvite
            catalogId={catalogId}
            guide={guide}
            onCopied={onCopied}
            roomName={roomName}
            rooms={rooms}
          />
        </Details>
      ) : null}
      <p className="agent-invite__note">{t('agentInvite.network.private.relay')}</p>
    </div>
  );
}

/**
 * 口令：一个按钮生成、把给 Agent 的整段话复制好。口令只在生成的那次响应里出现，所以已经有口令时
 * 只能换一个新的。
 */
function JoinCodeInvite({ catalogId, guide, roomName, rooms, onCopied }: InviteProps) {
  const { t } = useTranslation();
  const queryClient = useQueryClient();
  const access = usePrivateRoomAgentAccess(rooms, catalogId);
  const [generated, setGenerated] = useState<GeneratedJoinCode | null>(null);
  const [autoCopied, setAutoCopied] = useState(false);
  const queryKey = privateRoomAgentAccessQueryKey(catalogId);
  const messageFor = (code: string) =>
    t('privateRooms.governance.agentAccess.message', { code, guide, room: roomName });

  const mutation = useMutation({
    mutationFn: async () => await rooms.generateJoinCode(catalogId),
    onSuccess: async (result) => {
      if (!result.ok) return;
      setGenerated(result.value);
      // 和房间设置里的口令面板共用缓存：那边马上显示“口令已开启”，不闪回旧状态。
      queryClient.setQueryData<Result<PrivateRoomAgentAccess, PrivateRoomFailure>>(
        queryKey,
        (current) =>
          current?.ok === true
            ? ok({ ...current.value, joinCode: { createdAtUnixMs: result.value.createdAtUnixMs } })
            : current,
      );
      // 一次点击就复制好。有的浏览器等完服务器就不认那次点击了，这时下面照样有复制按钮。
      try {
        await navigator.clipboard.writeText(messageFor(result.value.code));
        setAutoCopied(true);
        onCopied?.();
      } catch {
        // 不算错。
      }
      await queryClient.invalidateQueries({ queryKey });
    },
  });

  const failure = mutation.data?.ok === false ? mutation.data.error : null;
  const forbidden = failure !== null && forbiddenCodes.has(failure.code);
  const existing = access.data?.ok === true && access.data.value.joinCode !== null;

  return (
    <>
      <p className="agent-invite__note">{t('agentInvite.network.code.detail')}</p>
      {forbidden ? (
        <Banner role={null} tone="info">
          {t('agentInvite.network.private.forbidden')}
        </Banner>
      ) : generated === null ? (
        <>
          <Button
            disabled={access.isPending || mutation.isPending}
            icon={mutation.isPending ? <Spinner /> : <KeyRound aria-hidden="true" />}
            onClick={() => {
              mutation.mutate();
            }}
            tone="ghost"
          >
            {t(
              existing
                ? 'agentInvite.network.private.replace'
                : 'agentInvite.network.private.create',
            )}
          </Button>
          {existing ? (
            <p className="agent-invite__note">{t('agentInvite.network.private.replaceNote')}</p>
          ) : null}
        </>
      ) : (
        <>
          {autoCopied ? (
            <Banner tone="success">{t('agentInvite.network.private.copied')}</Banner>
          ) : null}
          <CopyBlock
            copiedLabel={t('agentInvite.network.private.copied')}
            copyLabel={t('agentInvite.network.private.copy')}
            failedLabel={t('agentInvite.message.failed')}
            onCopied={onCopied}
            text={messageFor(generated.code)}
            textLabel={t('agentInvite.message.label')}
            tone="ghost"
          />
          <p className="agent-invite__note">{t('agentInvite.network.private.onlyOnce')}</p>
        </>
      )}
      {failure === null || forbidden ? null : <PrivateRoomFailureNotice failure={failure} />}
    </>
  );
}
