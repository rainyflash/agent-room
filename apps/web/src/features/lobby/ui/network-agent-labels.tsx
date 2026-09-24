import {
  createContext,
  useContext,
  useEffect,
  useSyncExternalStore,
  type PropsWithChildren,
} from 'react';

import type { NetworkAgentLabelStore } from '../application/network-agent-label-store';

const NetworkAgentLabelsContext = createContext<NetworkAgentLabelStore | null>(null);
const empty: ReadonlySet<string> = new Set();
const subscribeToNothing = () => () => undefined;
const nothing = () => empty;

export function NetworkAgentLabelsProvider({
  children,
  store,
}: PropsWithChildren<{ readonly store: NetworkAgentLabelStore }>) {
  return (
    <NetworkAgentLabelsContext.Provider value={store}>
      {children}
    </NetworkAgentLabelsContext.Provider>
  );
}

/** 这些 Agent 里哪些是网络 Agent。没有查询服务时（例如单独渲染的组件）一律当作不是。 */
export function useNetworkAgentIds(agentIds: readonly string[]): ReadonlySet<string> {
  const store = useContext(NetworkAgentLabelsContext);
  const key = agentIds.join(',');
  useEffect(() => {
    if (store !== null && key !== '') store.request(key.split(','));
  }, [store, key]);
  return useSyncExternalStore(
    store?.subscribe ?? subscribeToNothing,
    store?.getSnapshot ?? nothing,
    store?.getSnapshot ?? nothing,
  );
}

export function useIsNetworkAgent(agentId: string | null): boolean {
  const networkAgents = useNetworkAgentIds(agentId === null ? [] : [agentId]);
  return agentId !== null && networkAgents.has(agentId);
}
