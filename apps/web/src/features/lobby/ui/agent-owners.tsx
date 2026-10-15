import {
  createContext,
  useCallback,
  useContext,
  useEffect,
  useMemo,
  useSyncExternalStore,
  type PropsWithChildren,
} from 'react';
import { useTranslation } from 'react-i18next';

import type { AgentOwnerStore } from '../application/agent-owner-store';
import { characterLabel, OWNER_NAME_LIMIT } from '../scene/character-labels';
import type { AgentOwner } from '../domain/agent-owners';

const AgentOwnerStoreContext = createContext<AgentOwnerStore | null>(null);

type Scope = { readonly catalogId: string; readonly selfPrincipalId: string | null };
const AgentOwnerScopeContext = createContext<Scope | null>(null);

const noOwners: ReadonlyMap<string, AgentOwner> = new Map();
const subscribeToNothing = () => () => undefined;
const nothing = () => undefined;

/** 整个应用一份：登录以后才有，退出登录时由组合根清空。 */
export function AgentOwnersProvider({
  children,
  store,
}: PropsWithChildren<{ readonly store: AgentOwnerStore }>) {
  return (
    <AgentOwnerStoreContext.Provider value={store}>{children}</AgentOwnerStoreContext.Provider>
  );
}

/** 包住一个房间的界面：问的是这个房间里的 Agent，`selfPrincipalId` 用来认出“你的 Agent”。 */
export function AgentOwnersScope({
  catalogId,
  children,
  selfPrincipalId,
}: PropsWithChildren<Scope>) {
  return (
    <AgentOwnerScopeContext.Provider value={{ catalogId, selfPrincipalId }}>
      {children}
    </AgentOwnerScopeContext.Provider>
  );
}

/**
 * 这个房间里这些 Agent 是谁的。没有查询服务或者不在房间里（例如不登录的围观页、单独渲染的组件）
 * 时什么都不回答。
 */
export function useAgentOwners(agentIds: readonly string[]): ReadonlyMap<string, AgentOwner> {
  const store = useContext(AgentOwnerStoreContext);
  const scope = useContext(AgentOwnerScopeContext);
  const catalogId = scope?.catalogId ?? null;
  const key = agentIds.join(',');
  useEffect(() => {
    if (store !== null && catalogId !== null && key !== '') {
      store.request(catalogId, key.split(','));
    }
  }, [store, catalogId, key]);
  const snapshot = useSyncExternalStore(
    store?.subscribe ?? subscribeToNothing,
    store?.getSnapshot ?? nothing,
    store?.getSnapshot ?? nothing,
  );
  return catalogId === null ? noOwners : (snapshot?.get(catalogId) ?? noOwners);
}

export type AgentOwnerText = {
  /** 场景里名牌下的一行：“Ada 的 Agent”“你的 Agent”“网络 Agent · 由 Ada 放行”。不知道时为空。 */
  readonly caption: (owner: AgentOwner | undefined) => string | null;
  /**
   * 名册和详情里写的：那里已经另外标了网络 Agent，所以网络 Agent 只写是谁放行的。不知道、没有时为空。
   */
  readonly detail: (owner: AgentOwner | undefined) => string | null;
};

export function useAgentOwnerText(): AgentOwnerText {
  const { t } = useTranslation();
  const selfPrincipalId = useContext(AgentOwnerScopeContext)?.selfPrincipalId ?? null;
  /** 放行的人：你自己时为空名字。 */
  const admitter = useCallback(
    (owner: AgentOwner | undefined): { readonly you: boolean; readonly name: string } | null => {
      if (owner?.kind !== 'network' || owner.admittedBy === undefined) return null;
      return {
        you: owner.admittedBy.principalId === selfPrincipalId,
        name: shownName(owner.admittedBy.displayName, t('lobby.owner.someone')),
      };
    },
    [selfPrincipalId, t],
  );
  const local = useCallback(
    (owner: AgentOwner | undefined): string | null => {
      if (owner?.kind !== 'local' || owner.owner === undefined) return null;
      return owner.owner.principalId === selfPrincipalId
        ? t('lobby.owner.yours')
        : t('lobby.owner.of', {
            name: shownName(owner.owner.displayName, t('lobby.owner.someone')),
          });
    },
    [selfPrincipalId, t],
  );
  return useMemo(
    () => ({
      caption: (owner: AgentOwner | undefined) => {
        if (owner?.kind !== 'network') return local(owner);
        const by = admitter(owner);
        return by === null
          ? t('lobby.owner.network')
          : by.you
            ? t('lobby.owner.networkAdmittedByYou')
            : t('lobby.owner.networkAdmittedBy', { name: by.name });
      },
      detail: (owner: AgentOwner | undefined) => {
        if (owner?.kind !== 'network') return local(owner);
        const by = admitter(owner);
        return by === null
          ? null
          : by.you
            ? t('lobby.owner.admittedByYou')
            : t('lobby.owner.admittedBy', { name: by.name });
      },
    }),
    [admitter, local, t],
  );
}

/** 没有昵称的写“某人”；太长的截短，免得把后半句挤掉。 */
function shownName(name: string, fallback: string): string {
  const trimmed = name.trim();
  return trimmed.length > 0 ? characterLabel(trimmed, OWNER_NAME_LIMIT) : fallback;
}
