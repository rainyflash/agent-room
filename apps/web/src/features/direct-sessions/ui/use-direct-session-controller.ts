import { useMutation, useQueryClient } from '@tanstack/react-query';
import { useCallback, useMemo, useState } from 'react';

import { useAppServices } from '@/app/app-services';
import {
  directSessionListQueryKey,
  useDirectSessionList,
} from '@/features/direct-sessions/data/direct-session-queries';
import type {
  DirectAgent,
  DirectContact,
  DirectSession,
  DirectSessionFailure,
} from '@/features/direct-sessions/domain/direct-session';
import { ok, type Result } from '@/shared/result';

export type DirectSessionController = {
  readonly blocking: boolean;
  readonly clearFailure: () => void;
  readonly failure: DirectSessionFailure | null;
  readonly loading: boolean;
  readonly markDisplayed: (roomId: string, matrixEventId: string) => Promise<void>;
  readonly openAgent: (
    targetAgentId: string,
  ) => Promise<Result<DirectSession, DirectSessionFailure>>;
  readonly opening: boolean;
  readonly retry: () => Promise<void>;
  readonly sessions: readonly DirectSession[];
  readonly setBlocked: (
    target: DirectAgent,
    blocked: boolean,
  ) => Promise<Result<DirectContact, DirectSessionFailure>>;
};

export function useDirectSessionController(enabled: boolean): DirectSessionController {
  const { directSessionCoordinator, directSessions } = useAppServices();
  const queryClient = useQueryClient();
  const list = useDirectSessionList(directSessions, enabled);
  const [actionFailure, setActionFailure] = useState<DirectSessionFailure | null>(null);
  const [localBlockRevision, setLocalBlockRevision] = useState(0);
  const openMutation = useMutation({
    mutationFn: async (targetAgentId: string) => await directSessionCoordinator.open(targetAgentId),
  });
  const blockMutation = useMutation({
    mutationFn: async ({ blocked, target }: { blocked: boolean; target: DirectAgent }) =>
      await directSessionCoordinator.setBlocked(target, blocked),
  });
  const listedSessions = list.data?.ok === true ? list.data.value : [];
  const sessions = useMemo(
    () =>
      Object.freeze(
        listedSessions.map((session) =>
          directSessionCoordinator.isLocallyBlocked(session.target.agentId)
            ? projectLocallyBlocked(session)
            : session,
        ),
      ),
    [directSessionCoordinator, listedSessions, localBlockRevision],
  );
  const listFailure =
    list.data?.ok === false
      ? list.data.error
      : list.isError
        ? { code: 'direct_session.query_failed', retryable: true }
        : null;

  const refresh = useCallback(async (): Promise<void> => {
    await queryClient.invalidateQueries({ queryKey: directSessionListQueryKey });
  }, [queryClient]);

  const openAgent = useCallback(
    async (targetAgentId: string): Promise<Result<DirectSession, DirectSessionFailure>> => {
      setActionFailure(null);
      const result = await openMutation.mutateAsync(targetAgentId);
      if (!result.ok) {
        setActionFailure(result.error);
        return result;
      }
      queryClient.setQueryData(directSessionListQueryKey, (current: unknown) =>
        mergeSessionCache(current, result.value),
      );
      await refresh();
      return result;
    },
    [openMutation, queryClient, refresh],
  );

  const setBlocked = useCallback(
    async (
      target: DirectAgent,
      blocked: boolean,
    ): Promise<Result<DirectContact, DirectSessionFailure>> => {
      setActionFailure(null);
      const pending = blockMutation.mutateAsync({ blocked, target });
      if (blocked) {
        setLocalBlockRevision((revision) => revision + 1);
      }
      const result = await pending;
      if (!result.ok) {
        setActionFailure(result.error);
        return result;
      }
      setLocalBlockRevision((revision) => revision + 1);
      await refresh();
      return result;
    },
    [blockMutation, refresh],
  );

  const markDisplayed = useCallback(
    async (roomId: string, matrixEventId: string): Promise<void> => {
      const result = await directSessionCoordinator.markDisplayed(roomId, matrixEventId);
      if (!result.ok) {
        setActionFailure(result.error);
      }
    },
    [directSessionCoordinator],
  );

  return {
    blocking: blockMutation.isPending,
    clearFailure: () => {
      setActionFailure(null);
    },
    failure: actionFailure ?? listFailure,
    loading: enabled && list.isPending,
    markDisplayed,
    openAgent,
    opening: openMutation.isPending,
    retry: refresh,
    sessions,
    setBlocked,
  };
}

function projectLocallyBlocked(session: DirectSession): DirectSession {
  return Object.freeze({
    ...session,
    contactPolicy: Object.freeze({
      ...session.contactPolicy,
      deliveryAllowed: false,
      presenceDisclosure: 'hidden' as const,
      principalBlocksAgent: true,
    }),
  });
}

function mergeSessionCache(current: unknown, opened: DirectSession) {
  const sessions = isSuccessfulSessionList(current) ? current.value : [];
  return ok(
    Object.freeze([
      opened,
      ...sessions.filter((session) => session.catalogId !== opened.catalogId),
    ]),
  );
}

function isSuccessfulSessionList(
  value: unknown,
): value is Result<readonly DirectSession[], DirectSessionFailure> & { readonly ok: true } {
  return (
    typeof value === 'object' &&
    value !== null &&
    'ok' in value &&
    value.ok === true &&
    'value' in value &&
    Array.isArray(value.value)
  );
}
