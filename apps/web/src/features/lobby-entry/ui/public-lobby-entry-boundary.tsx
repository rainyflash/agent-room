import { Button, Spinner } from '@agent-room/ui-system';
import { MailOpen, RotateCw, TriangleAlert } from 'lucide-react';
import { useEffect } from 'react';
import { useTranslation } from 'react-i18next';
import { useQuery } from '@tanstack/react-query';

import { useAppServices } from '@/app/app-services';
import type { PublicLobbyRouteTarget } from '@/features/lobby-entry/domain/public-lobby-entry';
import { usePrivateRoomList } from '@/features/private-rooms/data/private-room-queries';
import { privateRoomAddressEntry } from '@/features/private-rooms/domain/private-room';
import { sessionStateName } from '@/features/session/ui/connection-model';
import { useSession } from '@/features/session/ui/session-provider';
import { EntryCard, EntryShell } from '@/shared/ui/entry-shell';

const PROVISIONING_BUSY_CODE = 'lobby.entry_provisioning_busy';
const NOT_FOUND_CODE = 'lobby.entry_not_found';
const PROVISIONING_REFRESH_MILLISECONDS = 1_500;

const entryCopy = {
  failure: {
    detail: 'lobbyEntry.failure.detail',
    title: 'lobbyEntry.failure.title',
  },
  invited: {
    detail: 'lobbyEntry.invited.detail',
    title: 'lobbyEntry.invited.title',
  },
  loading: {
    detail: 'lobbyEntry.loading.detail',
    title: 'lobbyEntry.loading.title',
  },
  missing: {
    detail: 'lobbyEntry.missing.detail',
    title: 'lobbyEntry.missing.title',
  },
  preparing: {
    detail: 'lobbyEntry.preparing.detail',
    title: 'lobbyEntry.preparing.title',
  },
} as const;

type EntryPhase = keyof typeof entryCopy;

export type PublicLobbyEntryBoundaryProps = {
  readonly catalogId: string;
  readonly onConnectionRequired: () => void;
  readonly onEntered: (target: PublicLobbyRouteTarget) => void;
};

export function PublicLobbyEntryBoundary({
  catalogId,
  onConnectionRequired,
  onEntered,
}: PublicLobbyEntryBoundaryProps) {
  const { t } = useTranslation();
  const { lobbyEntry, privateRooms } = useAppServices();
  const { snapshot } = useSession();
  const sessionState = sessionStateName(snapshot.value);
  const principalId = snapshot.context.principal?.principalId ?? null;
  const sessionReady = sessionState === 'ready' && principalId !== null;
  // 私人房间的地址也是 /lobby/<房间号>：先看是不是你的私人房间，是就直接打开，不拿去走公开大厅入场。
  // 房间列表读不出来时照公开大厅进，服务器对私人房间回“找不到”。
  const rooms = usePrivateRoomList(privateRooms, principalId ?? undefined, sessionReady);
  const privateEntry =
    principalId !== null && rooms.data?.ok === true
      ? privateRoomAddressEntry(rooms.data.value, catalogId, principalId)
      : null;
  const roomsSettled = rooms.data !== undefined || rooms.isError;
  const entry = useQuery({
    enabled: sessionReady && roomsSettled && privateEntry === null,
    networkMode: 'always',
    queryFn: async () => await lobbyEntry.enter(catalogId),
    queryKey: ['public-lobby-entry', catalogId] as const,
    refetchInterval: (query) =>
      query.state.data?.ok === false && query.state.data.error.code === PROVISIONING_BUSY_CODE
        ? PROVISIONING_REFRESH_MILLISECONDS
        : false,
    retry: false,
    staleTime: 0,
  });
  const target =
    privateEntry?.kind === 'joined'
      ? privateEntry
      : entry.data?.ok === true
        ? entry.data.value
        : null;
  const failureCode =
    entry.data?.ok === false
      ? entry.data.error.code
      : entry.isError
        ? 'lobby_entry.unexpected_failure'
        : null;
  const phase: EntryPhase =
    privateEntry?.kind === 'invited'
      ? 'invited'
      : failureCode === null
        ? 'loading'
        : failureCode === PROVISIONING_BUSY_CODE
          ? 'preparing'
          : failureCode === NOT_FOUND_CODE
            ? 'missing'
            : 'failure';
  const copy = entryCopy[phase];
  // 找不到、还没答复的邀请，重试都没用：只给回房间列表。
  const retryable = phase === 'preparing' || phase === 'failure';

  useEffect(() => {
    if (sessionState === 'unauthenticated' && snapshot.context.principal === null) {
      onConnectionRequired();
    }
  }, [onConnectionRequired, sessionState, snapshot.context.principal]);

  // 只认房间号和 Matrix 房间：私人房间的目标每次渲染都是新对象，别让它一遍遍跳转。
  const targetCatalogId = target?.catalogId ?? null;
  const targetRoomId = target?.matrixRoomId ?? null;
  useEffect(() => {
    if (targetCatalogId !== null && targetRoomId !== null) {
      onEntered({ catalogId: targetCatalogId, matrixRoomId: targetRoomId });
    }
  }, [onEntered, targetCatalogId, targetRoomId]);

  return (
    <EntryShell>
      <EntryCard
        actions={
          phase === 'loading' ? undefined : (
            <>
              {retryable ? (
                <Button
                  icon={<RotateCw aria-hidden="true" />}
                  onClick={() => void entry.refetch()}
                  size="large"
                  tone="primary"
                >
                  {t(phase === 'preparing' ? 'lobbyEntry.preparing.retry' : 'lobbyEntry.retry')}
                </Button>
              ) : null}
              <a
                className={`ar-button ar-button--large ${retryable ? 'ar-button--ghost' : 'ar-button--primary'}`}
                href="/rooms"
              >
                {t(phase === 'invited' ? 'lobbyEntry.invited.open' : 'entry.backToRooms')}
              </a>
            </>
          )
        }
        detail={t(copy.detail)}
        details={
          phase === 'failure' || phase === 'missing' ? (
            <dl>
              <div>
                <dt>{t('entry.errorCode')}</dt>
                <dd>
                  <code>{failureCode}</code>
                </dd>
              </div>
            </dl>
          ) : undefined
        }
        icon={
          phase === 'failure' || phase === 'missing' ? (
            <TriangleAlert />
          ) : phase === 'invited' ? (
            <MailOpen />
          ) : (
            <Spinner />
          )
        }
        title={t(copy.title)}
        tone={phase === 'failure' || phase === 'missing' ? 'alert' : 'neutral'}
      />
    </EntryShell>
  );
}
