import {
  createContext,
  useContext,
  useEffect,
  useSyncExternalStore,
  type PropsWithChildren,
} from 'react';

import { useTranslation } from 'react-i18next';

import type { NetworkAgentLabelStore } from '../application/network-agent-label-store';

const NetworkAgentLabelsContext = createContext<NetworkAgentLabelStore | null>(null);
/** 当前房间加密时，网络 Agent 的收发由服务器代办，标记里要说清楚。 */
const NetworkAgentRelayContext = createContext(false);
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

/** 包住一个房间的界面：`relayed` 为真表示这是加密的私人房间，网络 Agent 由服务器代收发。 */
export function NetworkAgentRelayProvider({
  children,
  relayed,
}: PropsWithChildren<{ readonly relayed: boolean }>) {
  return (
    <NetworkAgentRelayContext.Provider value={relayed}>
      {children}
    </NetworkAgentRelayContext.Provider>
  );
}

/** 网络 Agent 标记的文字与悬停说明；私人房间里多一句“服务器代收发”。 */
export function useNetworkAgentLabel(): { readonly label: string; readonly hint: string } {
  const { t } = useTranslation();
  const relayed = useContext(NetworkAgentRelayContext);
  return relayed
    ? { label: t('lobby.agent.networkRelayed'), hint: t('lobby.agent.networkRelayedHint') }
    : { label: t('lobby.agent.network'), hint: t('lobby.agent.networkHint') };
}
