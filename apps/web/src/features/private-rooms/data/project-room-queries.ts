import { queryOptions, useQuery } from '@tanstack/react-query';

import { privateRoomListQueryKey } from '@/features/private-rooms/data/private-room-queries';
import type { ProjectRoomGateway } from '@/features/private-rooms/domain/project-rooms';
import { err } from '@/shared/result';

/** 测试和夹具里没有这项服务时不去问；查询本来也不会跑。 */
const unavailable = { code: 'project_room.unavailable', retryable: false } as const;

/** 等着的认领，设置开着时隔一会儿再问一次：后台核对上了，这里跟着换成“绑上了”。 */
const PENDING_CLAIM_POLL_MS = 30_000;

/**
 * 这间房绑着哪个仓库（只有房主问得到）。挂在房间列表的键下面：改回私密、关掉都会解绑，列表刷新时这份
 * 跟着重问。房间设置里那一组和顶栏的勾读同一份。
 */
export const roomRepositoryQueryKey = (catalogId: string) =>
  [...privateRoomListQueryKey, 'repository', catalogId] as const;

export function useRoomRepository(
  gateway: ProjectRoomGateway | undefined,
  catalogId: string,
  enabled = true,
) {
  return useQuery(
    queryOptions({
      enabled: enabled && gateway !== undefined,
      queryKey: roomRepositoryQueryKey(catalogId),
      queryFn: async () =>
        gateway === undefined ? err(unavailable) : await gateway.binding(catalogId),
      networkMode: 'always',
      retry: false,
      staleTime: 30_000,
    }),
  );
}

export const repositoryClaimQueryKey = (claimId: string) =>
  ['control-plane', 'project-room-claims', claimId] as const;

export function useRepositoryClaim(
  gateway: ProjectRoomGateway | undefined,
  claimId: string | null,
) {
  return useQuery(
    queryOptions({
      enabled: gateway !== undefined && claimId !== null,
      queryKey: repositoryClaimQueryKey(claimId ?? 'none'),
      queryFn: async () =>
        gateway === undefined || claimId === null
          ? err(unavailable)
          : await gateway.inspectClaim(claimId),
      networkMode: 'always',
      retry: false,
      staleTime: 5_000,
      refetchInterval: (query) => {
        const data = query.state.data;
        return data?.ok === true && data.value.status === 'pending' ? PENDING_CLAIM_POLL_MS : false;
      },
    }),
  );
}

/**
 * 这间房在等的认领是哪一个，只记在这个浏览器里：服务器没有“这间房在等哪个认领”的接口。换了浏览器看到的是
 * “还没绑”，再填同一个仓库，服务器回的还是同一个认领（同一个人同一个仓库只有一个），不会多占名额。
 * 存不了（无痕窗口、不让存）时只在这次打开里有效。
 */
const claimKeyPrefix = 'agent-room.project-room-claim.';
const unsaved = new Map<string, string>();

function claimKey(principalId: string, catalogId: string): string {
  return `${claimKeyPrefix}${principalId}.${catalogId}`;
}

export function rememberedClaim(principalId: string, catalogId: string): string | null {
  const key = claimKey(principalId, catalogId);
  try {
    return window.localStorage.getItem(key) ?? unsaved.get(key) ?? null;
  } catch {
    return unsaved.get(key) ?? null;
  }
}

export function rememberClaim(principalId: string, catalogId: string, claimId: string): void {
  const key = claimKey(principalId, catalogId);
  try {
    window.localStorage.setItem(key, claimId);
    unsaved.delete(key);
  } catch {
    // 只在本次有效。
    unsaved.set(key, claimId);
  }
}

export function forgetClaim(principalId: string, catalogId: string): void {
  const key = claimKey(principalId, catalogId);
  unsaved.delete(key);
  try {
    window.localStorage.removeItem(key);
  } catch {
    // 删不掉就留着：下次问到认领没了时再删。
  }
}
