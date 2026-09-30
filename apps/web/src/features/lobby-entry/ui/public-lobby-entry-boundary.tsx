import { Button, Spinner } from '@agent-room/ui-system';
import { RotateCw, TriangleAlert } from 'lucide-react';
import { useEffect } from 'react';
import { useTranslation } from 'react-i18next';
import { useQuery } from '@tanstack/react-query';

import { useAppServices } from '@/app/app-services';
import type { PublicLobbyEntryTarget } from '@/features/lobby-entry/domain/public-lobby-entry';
import { sessionStateName } from '@/features/session/ui/connection-model';
import { useSession } from '@/features/session/ui/session-provider';
import { EntryCard, EntryShell } from '@/shared/ui/entry-shell';

const PROVISIONING_BUSY_CODE = 'lobby.entry_provisioning_busy';
const PROVISIONING_REFRESH_MILLISECONDS = 1_500;

const entryCopy = {
  failure: {
    detail: 'lobbyEntry.failure.detail',
    title: 'lobbyEntry.failure.title',
  },
  loading: {
    detail: 'lobbyEntry.loading.detail',
    title: 'lobbyEntry.loading.title',
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
  readonly onEntered: (target: PublicLobbyEntryTarget) => void;
};

export function PublicLobbyEntryBoundary({
  catalogId,
  onConnectionRequired,
  onEntered,
}: PublicLobbyEntryBoundaryProps) {
  const { t } = useTranslation();
  const { lobbyEntry } = useAppServices();
  const { snapshot } = useSession();
  const sessionState = sessionStateName(snapshot.value);
  const sessionReady = sessionState === 'ready' && snapshot.context.principal !== null;
  const entry = useQuery({
    enabled: sessionReady,
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
  const target = entry.data?.ok === true ? entry.data.value : null;
  const failureCode =
    entry.data?.ok === false
      ? entry.data.error.code
      : entry.isError
        ? 'lobby_entry.unexpected_failure'
        : null;
  const phase: EntryPhase =
    failureCode === null
      ? 'loading'
      : failureCode === PROVISIONING_BUSY_CODE
        ? 'preparing'
        : 'failure';
  const copy = entryCopy[phase];

  useEffect(() => {
    if (sessionState === 'unauthenticated' && snapshot.context.principal === null) {
      onConnectionRequired();
    }
  }, [onConnectionRequired, sessionState, snapshot.context.principal]);

  useEffect(() => {
    if (target !== null) onEntered(target);
  }, [onEntered, target]);

  return (
    <EntryShell>
      <EntryCard
        actions={
          phase === 'loading' ? undefined : (
            <>
              <Button
                icon={<RotateCw aria-hidden="true" />}
                onClick={() => void entry.refetch()}
                size="large"
                tone="primary"
              >
                {t(phase === 'preparing' ? 'lobbyEntry.preparing.retry' : 'lobbyEntry.retry')}
              </Button>
              <a className="ar-button ar-button--large ar-button--ghost" href="/rooms">
                {t('entry.backToRooms')}
              </a>
            </>
          )
        }
        detail={t(copy.detail)}
        details={
          phase === 'failure' ? (
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
        icon={phase === 'failure' ? <TriangleAlert /> : <Spinner />}
        title={t(copy.title)}
        tone={phase === 'failure' ? 'alert' : 'neutral'}
      />
    </EntryShell>
  );
}
