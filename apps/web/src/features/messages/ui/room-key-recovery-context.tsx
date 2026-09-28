import { createContext, useContext, useSyncExternalStore, type ReactNode } from 'react';

import type { RoomKeyRecoveryStatus } from '@/features/messages/adapters/matrix-room-key-recovery';

const noRecovery: RoomKeyRecoveryStatus = {
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

/** 这个房间里还在等 Agent 重发的会话数。 */
export function useRoomKeyRecoveryPending(roomId: string): number {
  const recovery = useContext(RoomKeyRecoveryContext);
  return useSyncExternalStore(
    (listener) => recovery.subscribe(listener),
    () => recovery.pending(roomId),
  );
}
