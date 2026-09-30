import { Banner, Button, CopyBlock, Spinner } from '@agent-room/ui-system';
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
import { PrivateRoomFailureNotice } from '@/features/private-rooms/ui/private-room-create-flow';
import { ok, type Result } from '@/shared/result';

/** 服务器拒绝非管理者查看或生成口令时的错误码。 */
const forbiddenCodes = new Set(['join_code.forbidden', 'private_room.forbidden']);

/**
 * 在“接入 Agent”里请网络 Agent 进私人房间：一个按钮生成口令、把给 Agent 的话复制好。
 * 口令只在生成的那次响应里出现，所以已经有口令时只能换一个新的；只有房间管理者能生成。
 */
export function PrivateRoomNetworkInvite({
  catalogId,
  guide,
  roomName,
  rooms,
  onCopied,
}: {
  readonly catalogId: string;
  readonly guide: string;
  readonly roomName: string;
  readonly rooms: PrivateRoomGateway;
  readonly onCopied?: (() => void) | undefined;
}) {
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

  const failure =
    mutation.data?.ok === false
      ? mutation.data.error
      : access.data?.ok === false
        ? access.data.error
        : null;
  const forbidden = failure !== null && forbiddenCodes.has(failure.code);
  const existing = access.data?.ok === true && access.data.value.joinCode !== null;

  return (
    <div className="agent-invite__private-code">
      <p className="agent-invite__note">
        {t('agentInvite.network.privateRoom', { room: roomName })}
      </p>
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
            size="large"
            tone="primary"
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
      <p className="agent-invite__note">{t('agentInvite.network.private.relay')}</p>
    </div>
  );
}
