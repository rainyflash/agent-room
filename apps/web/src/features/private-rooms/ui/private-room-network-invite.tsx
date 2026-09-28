import { Button } from '@agent-room/ui-system';
import { useMutation, useQueryClient } from '@tanstack/react-query';
import { AlertTriangle, Check, Copy, KeyRound, LoaderCircle } from 'lucide-react';
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

type CopyState = 'idle' | 'copied' | 'failed';

/**
 * 在「接入 Agent」对话框里请网络 Agent 进私人房间：一个按钮生成口令、把给 Agent 的话复制好。
 * 以前要先绕到房间设置里找「Agent 口令」，维护者看了一头雾水。口令只在生成的那次响应里出现，
 * 所以已经有口令时只能换一个新的；只有房间管理者能生成，服务器会拒绝别人。
 */
export function PrivateRoomNetworkInvite({
  catalogId,
  guide,
  roomName,
  rooms,
}: {
  readonly catalogId: string;
  readonly guide: string;
  readonly roomName: string;
  readonly rooms: PrivateRoomGateway;
}) {
  const { t } = useTranslation();
  const queryClient = useQueryClient();
  const access = usePrivateRoomAgentAccess(rooms, catalogId);
  const [generated, setGenerated] = useState<GeneratedJoinCode | null>(null);
  const [copyState, setCopyState] = useState<CopyState>('idle');
  const queryKey = privateRoomAgentAccessQueryKey(catalogId);

  const message =
    generated === null
      ? null
      : t('privateRooms.governance.agentAccess.message', {
          code: generated.code,
          guide,
          room: roomName,
        });

  /** 自动复制失败不算错：有的浏览器等完服务器就不认那次点击了，这时照样给出这段话和复制按钮。 */
  const copy = async (text: string, manual: boolean): Promise<void> => {
    try {
      await navigator.clipboard.writeText(text);
      setCopyState('copied');
    } catch {
      setCopyState(manual ? 'failed' : 'idle');
    }
  };

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
      // 一次点击就复制好。
      await copy(
        t('privateRooms.governance.agentAccess.message', {
          code: result.value.code,
          guide,
          room: roomName,
        }),
        false,
      );
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
      <p>{t('agentInvite.network.privateRoom', { room: roomName })}</p>
      {forbidden ? (
        <p className="agent-invite__note" role="status">
          {t('agentInvite.network.private.forbidden')}
        </p>
      ) : message === null ? (
        <>
          <p>{t('agentInvite.network.private.howTo')}</p>
          <Button
            disabled={access.isPending || mutation.isPending}
            icon={
              mutation.isPending ? (
                <LoaderCircle aria-hidden="true" className="private-room-spin" />
              ) : (
                <KeyRound aria-hidden="true" />
              )
            }
            onClick={() => {
              setCopyState('idle');
              mutation.mutate();
            }}
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
          <pre className="agent-invite__network-prompt">{message}</pre>
          <Button
            icon={
              copyState === 'copied' ? <Check aria-hidden="true" /> : <Copy aria-hidden="true" />
            }
            onClick={() => void copy(message, true)}
            tone={copyState === 'failed' ? 'alert' : 'quiet'}
          >
            {t(
              copyState === 'copied'
                ? 'agentInvite.network.private.copied'
                : 'agentInvite.network.private.copy',
            )}
          </Button>
          {copyState === 'failed' ? (
            <p className="agent-invite__error" role="status">
              <AlertTriangle aria-hidden="true" />
              {t('agentInvite.network.private.copyFailed')}
            </p>
          ) : null}
          <p className="agent-invite__note">{t('agentInvite.network.private.onlyOnce')}</p>
        </>
      )}
      {failure === null || forbidden ? null : <PrivateRoomFailureNotice failure={failure} />}
      <p className="agent-invite__note">{t('agentInvite.network.private.relay')}</p>
    </div>
  );
}
