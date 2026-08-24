import { useMachine } from '@xstate/react';
import { createContext, useContext, useEffect, useMemo, type PropsWithChildren } from 'react';
import type { SnapshotFrom } from 'xstate';

import {
  createSessionMachine,
  type SessionEvent,
  type SessionMachine,
} from '@/features/session/domain/session-machine';
import type { SessionDependencies } from '@/features/session/domain/session';

type SessionContextValue = {
  readonly send: (event: SessionEvent) => void;
  readonly snapshot: SnapshotFrom<SessionMachine>;
};

const SessionReactContext = createContext<SessionContextValue | null>(null);

export type SessionProviderProps = PropsWithChildren<{
  readonly dependencies: SessionDependencies;
}>;

export function SessionProvider({ children, dependencies }: SessionProviderProps) {
  const machine = useMemo(() => createSessionMachine(dependencies), [dependencies]);
  const [snapshot, send] = useMachine(machine);

  useEffect(() => {
    const handleOffline = (): void => {
      send({ type: 'OFFLINE' });
    };
    const handleOnline = (): void => {
      send({ type: 'ONLINE' });
    };
    window.addEventListener('offline', handleOffline);
    window.addEventListener('online', handleOnline);
    return () => {
      window.removeEventListener('offline', handleOffline);
      window.removeEventListener('online', handleOnline);
    };
  }, [send]);

  useEffect(() => {
    const connection = snapshot.context.connection;
    if (connection === null) {
      return undefined;
    }
    return connection.observe((status) => {
      const events: Partial<Record<typeof status, SessionEvent>> = {
        failed: { type: 'MATRIX_INTERRUPTED' },
        ready: { type: 'MATRIX_RESTORED' },
        reconnecting: { type: 'MATRIX_INTERRUPTED' },
        stopped: { type: 'MATRIX_INTERRUPTED' },
      };
      const event = events[status];
      if (event !== undefined) {
        send(event);
      }
    });
  }, [send, snapshot.context.connection]);

  const value = useMemo<SessionContextValue>(() => ({ send, snapshot }), [send, snapshot]);
  return <SessionReactContext.Provider value={value}>{children}</SessionReactContext.Provider>;
}

export function useSession(): SessionContextValue {
  const value = useContext(SessionReactContext);
  if (value === null) {
    throw new Error('SessionProvider is missing.');
  }
  return value;
}
