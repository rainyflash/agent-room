import {
  createContext,
  useContext,
  useMemo,
  useSyncExternalStore,
  type PropsWithChildren,
} from 'react';
import { PersonalWorkspaceStore } from '../application/personal-workspace-store';

type WorkspaceContextValue = {
  readonly snapshot: ReturnType<PersonalWorkspaceStore['getSnapshot']>;
  readonly change: PersonalWorkspaceStore['change'];
  readonly changeForAccount: PersonalWorkspaceStore['changeForAccount'];
  readonly changeMany: PersonalWorkspaceStore['changeMany'];
  readonly retry: PersonalWorkspaceStore['retry'];
  readonly toggleFavorite: PersonalWorkspaceStore['toggleFavorite'];
  readonly undoFavorite: PersonalWorkspaceStore['undoFavorite'];
};
const Context = createContext<WorkspaceContextValue | null>(null);

export function PersonalWorkspaceProvider({
  store,
  children,
}: PropsWithChildren<{ readonly store: PersonalWorkspaceStore }>) {
  const snapshot = useSyncExternalStore(store.subscribe, store.getSnapshot, store.getSnapshot);
  const value = useMemo(
    () => ({
      snapshot,
      change: store.change,
      changeForAccount: store.changeForAccount,
      changeMany: store.changeMany,
      retry: store.retry,
      toggleFavorite: store.toggleFavorite,
      undoFavorite: store.undoFavorite,
    }),
    [snapshot, store],
  );
  return <Context.Provider value={value}>{children}</Context.Provider>;
}

export function usePersonalWorkspace(): WorkspaceContextValue | null {
  return useContext(Context);
}
