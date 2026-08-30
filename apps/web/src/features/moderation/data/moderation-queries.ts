import { queryOptions, useQuery } from '@tanstack/react-query';

import type { ModerationGateway } from '@/features/moderation/domain/moderation';

export const moderationCaseListQueryKey = ['control-plane', 'moderation', 'cases'] as const;
export const moderationActionListQueryKey = (catalogId: string) =>
  ['control-plane', 'moderation', 'actions', catalogId] as const;
export const moderationAuditListQueryKey = (catalogId: string) =>
  ['control-plane', 'moderation', 'audit', catalogId] as const;
export const moderationRoomCaseListQueryKey = (catalogId: string) =>
  ['control-plane', 'moderation', 'room-cases', catalogId] as const;
export const moderationCapabilitiesQueryKey = (catalogId: string) =>
  ['control-plane', 'moderation', 'capabilities', catalogId] as const;

export function moderationCaseListQueryOptions(gateway: ModerationGateway) {
  return queryOptions({
    queryFn: async () => await gateway.listCases(),
    queryKey: moderationCaseListQueryKey,
    networkMode: 'always',
    retry: false,
    staleTime: 5_000,
  });
}

export function moderationActionListQueryOptions(gateway: ModerationGateway, catalogId: string) {
  return queryOptions({
    queryFn: async () => await gateway.listActions(catalogId),
    queryKey: moderationActionListQueryKey(catalogId),
    networkMode: 'always',
    retry: false,
    staleTime: 5_000,
  });
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

export function useModerationCases(gateway: ModerationGateway, enabled = true) {
  return useQuery({ ...moderationCaseListQueryOptions(gateway), enabled });
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
