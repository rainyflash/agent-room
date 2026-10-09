import { Spinner } from '@agent-room/ui-system';
import { useQuery } from '@tanstack/react-query';
import { Link } from '@tanstack/react-router';
import { AlertTriangle } from 'lucide-react';
import { useTranslation } from 'react-i18next';

import { useAppServices } from '@/app/app-services';
import { PublicWatchError, usePublicWatch } from '@/features/public-watch/data/public-watch-query';
import {
  defaultWatchSlug,
  isFinalWatchFailure,
  type PublicWatchFailure,
} from '@/features/public-watch/domain/public-watch';
import { EntryCard, EntryShell } from '@/shared/ui/entry-shell';
import { PublicWatchView, type WatchAccount } from './public-watch-view';

/**
 * `/watch` 和 `/watch/<slug>`：不登录也能看公共大厅。这页和首页一样不在登录会话里，没登录的人
 * 打开它不会去清理登录状态；登录了没有自己问一下服务器。
 */
export function PublicWatchPage({ slug }: { readonly slug: string | null }) {
  const { t } = useTranslation();
  const { config, controlPlane, publicWatch } = useAppServices();
  const query = usePublicWatch(publicWatch, slug ?? defaultWatchSlug);
  const session = useQuery({
    networkMode: 'always',
    queryFn: async () => await controlPlane.readSession(),
    // 和首页同一个键：从首页点进来不用再问一次。
    queryKey: ['landing', 'session'] as const,
    retry: false,
    staleTime: 60_000,
  });
  const failure = watchFailure(query.error);
  // 大厅里的话是别人随口说的，不该被搜到；robots.txt 里也挡着。
  const noindex = <meta content="noindex" name="robots" />;

  if (failure !== null && (query.data === undefined || isFinalWatchFailure(failure))) {
    return (
      <>
        {noindex}
        <WatchFailureCard
          failure={failure}
          onRetry={() => {
            void query.refetch();
          }}
          slug={slug}
        />
      </>
    );
  }
  if (query.data === undefined) {
    return (
      <>
        {noindex}
        <EntryShell>
          <EntryCard icon={<Spinner />} title={t('publicWatch.loading')} />
        </EntryShell>
      </>
    );
  }
  const lobbyPath = `/lobby/${query.data.lobby.catalogId}`;
  const account: WatchAccount =
    session.data?.ok === true
      ? { kind: 'signed-in' }
      : session.isError || session.data?.ok === false
        ? {
            kind: 'signed-out',
            registrationOpen: config.registrationMode === 'open-email',
            // 登录或注册完直接回到这个大厅，进去就能说话。
            onRegister: () => {
              void controlPlane.beginAuthentication(lobbyPath, 'register');
            },
            onSignIn: () => {
              void controlPlane.beginAuthentication(lobbyPath, 'sign-in');
            },
          }
        : { kind: 'unknown' };
  return (
    <>
      {noindex}
      <PublicWatchView
        account={account}
        defaultLobby={slug === null}
        stale={failure !== null}
        watch={query.data}
      />
    </>
  );
}

function watchFailure(error: Error | null): PublicWatchFailure | null {
  if (error === null) return null;
  return error instanceof PublicWatchError
    ? error.failure
    : { code: 'public_watch.unreachable', retryable: true };
}

function WatchFailureCard({
  failure,
  onRetry,
  slug,
}: {
  readonly failure: PublicWatchFailure;
  readonly onRetry: () => void;
  readonly slug: string | null;
}) {
  const { t } = useTranslation();
  const home = (
    <Link className="ar-button ar-button--large ar-button--ghost" to="/">
      {t('publicWatch.home')}
    </Link>
  );
  const details = (
    <dl>
      <div>
        <dt>{t('entry.errorCode')}</dt>
        <dd>
          <code>{failure.code}</code>
        </dd>
      </div>
    </dl>
  );
  if (failure.code === 'public_watch.disabled') {
    return (
      <EntryShell>
        <EntryCard
          actions={home}
          detail={t('publicWatch.disabled.detail')}
          details={details}
          icon={<AlertTriangle />}
          title={t('publicWatch.disabled.title')}
          tone="alert"
        />
      </EntryShell>
    );
  }
  if (failure.code === 'public_watch.lobby_not_found') {
    return (
      <EntryShell>
        <EntryCard
          actions={
            slug === null ? (
              home
            ) : (
              <>
                <Link className="ar-button ar-button--large ar-button--primary" to="/watch">
                  {t('publicWatch.notFound.action')}
                </Link>
                {home}
              </>
            )
          }
          detail={t('publicWatch.notFound.detail')}
          details={details}
          icon={<AlertTriangle />}
          title={t('publicWatch.notFound.title')}
          tone="alert"
        />
      </EntryShell>
    );
  }
  return (
    <EntryShell>
      <EntryCard
        actions={
          <>
            <button
              className="ar-button ar-button--large ar-button--primary"
              onClick={onRetry}
              type="button"
            >
              {t('publicWatch.retry')}
            </button>
            {home}
          </>
        }
        detail={t('publicWatch.unavailable.detail')}
        details={details}
        icon={<AlertTriangle />}
        title={t('publicWatch.unavailable.title')}
        tone="alert"
      />
    </EntryShell>
  );
}
