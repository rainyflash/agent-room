import { createContext, useContext, useSyncExternalStore, type ReactNode } from 'react';

import type { RoomKeyRecoveryStatus } from '@/features/messages/adapters/matrix-room-key-recovery';
import type { UndecryptableRecovery } from '@/features/messages/domain/message';

const noRecovery: RoomKeyRecoveryStatus = {
  awaitingSigning: () => 0,
  pending: () => 0,
  subscribe: () => () => undefined,
};

const RoomKeyRecoveryContext = createContext<RoomKeyRecoveryStatus>(noRecovery);

/** 找回历史的状态：哪些房间还在等 Agent 重发密钥。没有提供时（测试、未登录）一律当作没有。 */
export function RoomKeyRecoveryProvider({
  children,
  recovery,
}: {
  readonly children: ReactNode;
  readonly recovery: RoomKeyRecoveryStatus;
}) {
  return (
    <RoomKeyRecoveryContext.Provider value={recovery}>{children}</RoomKeyRecoveryContext.Provider>
  );
}

/** 这个房间里解不开的消息找回到了哪一步。 */
export function useRoomKeyRecovery(roomId: string): UndecryptableRecovery {
  const recovery = useContext(RoomKeyRecoveryContext);
  return useSyncExternalStore(
    (listener) => recovery.subscribe(listener),
    () => recoveryState(recovery, roomId),
  );
}

function recoveryState(recovery: RoomKeyRecoveryStatus, roomId: string): UndecryptableRecovery {
  if (recovery.pending(roomId) > 0) return 'requested';
  return recovery.awaitingSigning(roomId) > 0 ? 'awaiting_signing' : 'idle';
}
