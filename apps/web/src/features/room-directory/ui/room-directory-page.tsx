import { Button } from '@agent-room/ui-system';
import { Link } from '@tanstack/react-router';
import {
  ArrowRight,
  Building2,
  CircleAlert,
  CloudOff,
  LayoutGrid,
  LoaderCircle,
  Radio,
  RefreshCw,
  ShieldCheck,
  UsersRound,
} from 'lucide-react';
import { motion, useReducedMotion } from 'motion/react';
import type { ReactNode } from 'react';
import { useTranslation } from 'react-i18next';

import { useAppServices } from '@/app/app-services';
import { LanguageControl } from '@/features/preferences/ui/language-control';
import { usePublicRoomDirectory } from '@/features/room-directory/data/public-room-directory-query';
import type { PublicRoomSummary } from '@/features/room-directory/domain/public-room-directory';

import './room-directory-page.css';

export function RoomDirectoryPage() {
  const { roomDirectory } = useAppServices();
  const query = usePublicRoomDirectory(roomDirectory);
  const result = query.data;

  return (
    <RoomDirectoryView
      failureCode={result?.ok === false ? result.error.code : null}
      loading={query.isPending}
      onRefresh={() => void query.refetch()}
      rooms={result?.ok === true ? result.value : []}
    />
  );
}

export type RoomDirectoryViewProps = {
  readonly failureCode: string | null;
  readonly loading: boolean;
  readonly onRefresh: () => void;
  readonly rooms: readonly PublicRoomSummary[];
};

export function RoomDirectoryView({
  failureCode,
  loading,
  onRefresh,
  rooms,
}: RoomDirectoryViewProps) {
  const { t } = useTranslation();
  const reduceMotion = useReducedMotion();

  return (
    <main className="room-directory" id="main-content">
      <header className="room-directory__topbar">
        <a aria-label={t('app.name')} className="room-directory__brand" href="/">
          <img alt="" src="/agent-room-mark.svg" />
          <span>{t('app.name')}</span>
        </a>
        <nav aria-label={t('roomDirectory.title')} className="room-directory__nav">
          <Link to="/workspace">
            <LayoutGrid aria-hidden="true" />
            <span>{t('roomDirectory.workspace')}</span>
          </Link>
          <Link params={{ section: 'security' }} to="/settings/$section">
            <ShieldCheck aria-hidden="true" />
            <span>{t('roomDirectory.security')}</span>
          </Link>
          <LanguageControl />
        </nav>
      </header>

      <section className="room-directory__hero">
        <div>
          <p className="eyebrow">{t('roomDirectory.eyebrow')}</p>
          <h1>{t('roomDirectory.title')}</h1>
          <p>{t('roomDirectory.description')}</p>
        </div>
        <Button icon={<RefreshCw aria-hidden="true" />} onClick={onRefresh} tone="quiet">
          {t('roomDirectory.refresh')}
        </Button>
      </section>

      {loading ? (
        <DirectoryBoundary
          detail={t('roomDirectory.description')}
          icon={<LoaderCircle aria-hidden="true" className="room-directory__spin" />}
          role="status"
          title={t('roomDirectory.loading')}
        />
      ) : null}

      {!loading && failureCode !== null ? (
        <DirectoryBoundary
          action={
            <Button icon={<RefreshCw aria-hidden="true" />} onClick={onRefresh} tone="alert">
              {t('roomDirectory.refresh')}
            </Button>
          }
          detail={t('roomDirectory.failed.detail')}
          icon={<CircleAlert aria-hidden="true" />}
          role="alert"
          title={t('roomDirectory.failed.title')}
        >
          <code>{failureCode}</code>
        </DirectoryBoundary>
      ) : null}

      {!loading && failureCode === null && rooms.length === 0 ? (
        <DirectoryBoundary
          detail={t('roomDirectory.empty.detail')}
          icon={<CloudOff aria-hidden="true" />}
          role="status"
          title={t('roomDirectory.empty.title')}
        />
      ) : null}

      {!loading && failureCode === null && rooms.length > 0 ? (
        <section aria-label={t('roomDirectory.title')} className="room-directory__grid">
          {rooms.map((room, index) => (
            <motion.article
              animate={{ opacity: 1, y: 0 }}
              className="room-card"
              initial={reduceMotion ? false : { opacity: 0, y: 14 }}
              key={room.catalogId}
              transition={{ damping: 26, delay: index * 0.045, stiffness: 260, type: 'spring' }}
            >
              <header>
                <span className="room-card__signal">
                  <Radio aria-hidden="true" />
                </span>
                <div>
                  <p>{room.slug ?? room.catalogId}</p>
                  <h2>{room.name}</h2>
                </div>
              </header>
              <p className="room-card__description">{room.description}</p>
              <dl>
                <RoomFact
                  icon={<UsersRound aria-hidden="true" />}
                  label={t('roomDirectory.onlineAgents', { count: room.onlineAgentCount })}
                />
                <RoomFact
                  icon={<Building2 aria-hidden="true" />}
                  label={t('roomDirectory.activeInstances', {
                    count: room.activeInstanceCount,
                  })}
                />
              </dl>
              <footer>
                <span>
                  {t('roomDirectory.language')}: {room.language ?? t('roomDirectory.anyLanguage')}
                </span>
                <Link params={{ catalogId: room.catalogId }} search={{}} to="/lobby/$catalogId">
                  <span>{t('roomDirectory.enter')}</span>
                  <ArrowRight aria-hidden="true" />
                </Link>
              </footer>
            </motion.article>
          ))}
        </section>
      ) : null}
    </main>
  );
}

function RoomFact({ icon, label }: { readonly icon: ReactNode; readonly label: string }) {
  return (
    <div>
      <dt>{icon}</dt>
      <dd>{label}</dd>
    </div>
  );
}

function DirectoryBoundary({
  action,
  children,
  detail,
  icon,
  role,
  title,
}: {
  readonly action?: ReactNode;
  readonly children?: ReactNode;
  readonly detail: string;
  readonly icon: ReactNode;
  readonly role: 'alert' | 'status';
  readonly title: string;
}) {
  return (
    <section className="room-directory__boundary" role={role}>
      <span>{icon}</span>
      <div>
        <h2>{title}</h2>
        <p>{detail}</p>
        {children}
      </div>
      {action}
    </section>
  );
}
