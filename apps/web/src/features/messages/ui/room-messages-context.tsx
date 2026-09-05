import { createContext, useContext, useMemo, useSyncExternalStore, type ReactNode } from 'react';
import { useAppServices } from '@/app/app-services';
import { MessageRoomStore } from '../application/message-room-store';

const RoomMessagesContext = createContext<MessageRoomStore | null>(null);

export function RoomMessagesProvider({
  store,
  children,
}: {
  readonly store: MessageRoomStore;
  readonly children: ReactNode;
}) {
  return <RoomMessagesContext.Provider value={store}>{children}</RoomMessagesContext.Provider>;
}

export function useRoomMessages(roomId: string) {
  const shared = useContext(RoomMessagesContext);
  const { messages } = useAppServices();
  const store = useMemo(
    () => (shared?.roomId === roomId ? shared : new MessageRoomStore(messages, roomId)),
    [messages, roomId, shared],
  );
  const state = useSyncExternalStore(store.subscribe, store.getSnapshot, store.getSnapshot);
  return { state, store };
}
