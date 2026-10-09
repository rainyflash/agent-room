import { queryOptions, useQuery } from '@tanstack/react-query';

import {
  moderationActionDisplayStatus,
  type ModerationAction,
  type ModerationFailure,
  type ModerationGateway,
} from '@/features/moderation/domain/moderation';
import type { Result } from '@/shared/result';

/** 台账上有正在解除的动作时，隔多久再读一次。服务器每 30 秒解除一轮。 */
const ENDING_REFETCH_MS = 15_000;

export const moderationCaseListQueryKey = ['control-plane', 'moderation', 'cases'] as const;
export const moderationActionListQueryKey = (catalogId: string) =>
  ['control-plane', 'moderation', 'actions', catalogId] as const;
export const moderationAuditListQueryKey = (catalogId: string) =>
  ['control-plane', 'moderation', 'audit', catalogId] as const;
export const moderationRoomCaseListQueryKey = (catalogId: string) =>
  ['control-plane', 'moderation', 'room-cases', catalogId] as const;
export const moderationCapabilitiesQueryKey = (catalogId: string) =>
  ['control-plane', 'moderation', 'capabilities', catalogId] as const;

export function moderationActionListQueryOptions(gateway: ModerationGateway, catalogId: string) {
  return queryOptions({
    queryFn: async () => await gateway.listActions(catalogId),
    queryKey: moderationActionListQueryKey(catalogId),
    networkMode: 'always',
    refetchInterval: (query) => endingRefetchInterval(query.state.data, Date.now()),
    retry: false,
    staleTime: 5_000,
  });
}

/** 有过了期限、服务器还没解除的，隔一会儿再读，等它解除；都定下来了就不再读。 */
export function endingRefetchInterval(
  result: Result<readonly ModerationAction[], ModerationFailure> | undefined,
  now: number,
): number | false {
  return result?.ok === true &&
    result.value.some((action) => moderationActionDisplayStatus(action, now) === 'ending')
    ? ENDING_REFETCH_MS
    : false;
}

export function moderationRoomCaseListQueryOptions(gateway: ModerationGateway, catalogId: string) {
  return queryOptions({
    queryFn: async () => await gateway.listRoomCases(catalogId),
    queryKey: moderationRoomCaseListQueryKey(catalogId),
    networkMode: 'always',
    retry: false,
    staleTime: 5_000,
  });
}

export function moderationAuditListQueryOptions(gateway: ModerationGateway, catalogId: string) {
  return queryOptions({
    queryFn: async () => await gateway.listAudit(catalogId),
    queryKey: moderationAuditListQueryKey(catalogId),
    networkMode: 'always',
    retry: false,
    staleTime: 5_000,
  });
}

export function moderationCapabilitiesQueryOptions(gateway: ModerationGateway, catalogId: string) {
  return queryOptions({
    queryFn: async () => await gateway.inspectCapabilities(catalogId),
    queryKey: moderationCapabilitiesQueryKey(catalogId),
    networkMode: 'always',
    retry: false,
    staleTime: 5_000,
  });
}

export function useModerationActions(
  gateway: ModerationGateway,
  catalogId: string,
  enabled = true,
) {
  return useQuery({ ...moderationActionListQueryOptions(gateway, catalogId), enabled });
}

export function useModerationRoomCases(
  gateway: ModerationGateway,
  catalogId: string,
  enabled = true,
) {
  return useQuery({ ...moderationRoomCaseListQueryOptions(gateway, catalogId), enabled });
}

export function useModerationAudit(gateway: ModerationGateway, catalogId: string, enabled = true) {
  return useQuery({ ...moderationAuditListQueryOptions(gateway, catalogId), enabled });
}

export function useModerationCapabilities(gateway: ModerationGateway, catalogId: string) {
  return useQuery(moderationCapabilitiesQueryOptions(gateway, catalogId));
}
