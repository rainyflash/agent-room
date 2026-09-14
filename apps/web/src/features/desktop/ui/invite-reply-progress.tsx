import { useMemo, useSyncExternalStore } from 'react';
import { useTranslation } from 'react-i18next';
import { MessageRoomStore } from '@/features/messages/application/message-room-store';
import type { MessageGateway } from '@/features/messages/domain/message';
import { inviteReplyProgress } from '../domain/invite-reply';

export function InviteReplyProgress({
  gateway,
  agentId,
  roomId,
  principalId,
  startedAt,
}: {
  readonly gateway: MessageGateway;
  readonly agentId: string;
  readonly roomId: string;
  readonly principalId: string;
  readonly startedAt: number;
}) {
  const { t } = useTranslation();
  const store = useMemo(() => new MessageRoomStore(gateway, roomId), [gateway, roomId]);
  const state = useSyncExternalStore(store.subscribe, store.getSnapshot, store.getSnapshot);
  const progress =
    state.kind === 'ready'
      ? inviteReplyProgress(state.room.messages, { agentId, roomId, principalId, startedAt })
      : 'unavailable';
  return (
    <p className="agent-invite__reply-progress" role="status" data-progress={progress}>
      {t(`agentInvite.firstReply.${progress}`)}
    </p>
  );
}
