import { Button } from '@agent-room/ui-system';
import { LoaderCircle, RadioTower, RotateCw } from 'lucide-react';
import { useEffect } from 'react';
import { useTranslation } from 'react-i18next';
import { useQuery } from '@tanstack/react-query';

import { useAppServices } from '@/app/app-services';
import type { PublicLobbyEntryTarget } from '@/features/lobby-entry/domain/public-lobby-entry';
import { LanguageControl } from '@/features/preferences/ui/language-control';
import { sessionStateName } from '@/features/session/ui/connection-model';
import { useSession } from '@/features/session/ui/session-provider';

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

  useEffect(() => {
    if (sessionState === 'unauthenticated' && snapshot.context.principal === null) {
      onConnectionRequired();
    }
  }, [onConnectionRequired, sessionState, snapshot.context.principal]);

  useEffect(() => {
    if (target !== null) onEntered(target);
  }, [onEntered, target]);

  return (
    <main className="lobby-boundary" id="main-content">
      <header className="lobby-boundary__topbar">
        <a aria-label={t('app.name')} className="brand-lockup brand-lockup--ink" href="/connect">
          <img alt="" src="/agent-room-mark.svg" />
          <span>{t('app.name')}</span>
        </a>
        <LanguageControl />
      </header>
      <section aria-live="polite" className="lobby-boundary__body">
        {failureCode === null ? (
          <LoaderCircle aria-hidden="true" className="lobby-boundary__icon lobby-boundary__spin" />
        ) : (
          <RadioTower aria-hidden="true" className="lobby-boundary__icon" />
        )}
        <p className="eyebrow">{t('lobbyEntry.eyebrow')}</p>
        <h1>{t(failureCode === null ? 'lobbyEntry.loading.title' : 'lobbyEntry.failure.title')}</h1>
        <p>{t(failureCode === null ? 'lobbyEntry.loading.detail' : 'lobbyEntry.failure.detail')}</p>
        {failureCode === null ? null : (
          <div className="lobby-boundary__actions">
            <Button
              icon={<RotateCw aria-hidden="true" />}
              onClick={() => void entry.refetch()}
              tone="primary"
            >
              {t('lobbyEntry.retry')}
            </Button>
            <a className="lobby-boundary__link" href="/connect">
              {t('lobbyEntry.connect')}
            </a>
            <code>{failureCode}</code>
          </div>
        )}
      </section>
    </main>
  );
}
