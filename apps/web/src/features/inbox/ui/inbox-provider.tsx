import {
  createContext,
  useContext,
  useMemo,
  useSyncExternalStore,
  type PropsWithChildren,
} from 'react';
import type { InboxStore } from '../application/inbox-store';

type InboxContext = {
  readonly snapshot: ReturnType<InboxStore['getSnapshot']>;
  readonly refresh: InboxStore['refresh'];
};
const Context = createContext<InboxContext | null>(null);
export function InboxProvider({
  store,
  children,
}: PropsWithChildren<{ readonly store: InboxStore }>) {
  const snapshot = useSyncExternalStore(store.subscribe, store.getSnapshot, store.getSnapshot);
  const value = useMemo(() => ({ snapshot, refresh: store.refresh }), [snapshot, store]);
  return <Context.Provider value={value}>{children}</Context.Provider>;
}
export function useInbox(): InboxContext | null {
  return useContext(Context);
}
