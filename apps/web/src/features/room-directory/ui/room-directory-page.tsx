import { Button } from '@agent-room/ui-system';
import { Link } from '@tanstack/react-router';
import {
  ArrowRight,
  Building2,
  CircleAlert,
  CloudOff,
  LoaderCircle,
  RefreshCw,
  UsersRound,
  Search,
} from 'lucide-react';
import { motion, useReducedMotion } from 'motion/react';
import { useState, type ReactNode } from 'react';
import { useTranslation } from 'react-i18next';

import { useAppServices } from '@/app/app-services';
import { AppNavigation } from '@/shared/ui/app-navigation';
import { RoomIllustration } from '@/features/lobby/ui/room-illustration';
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
  const [search, setSearch] = useState('');
  const term = search.trim().toLowerCase();
  const visibleRooms = rooms.filter((room) =>
    [room.name, room.description, room.language, room.slug].some((value) =>
      value?.toLowerCase().includes(term),
    ),
  );

  return (
    <main className="room-directory" id="main-content">
      <AppNavigation active="rooms" />

      <section className="room-directory__hero">
        <div>
          <h1>{t('roomDirectory.title')}</h1>
          <p>{t('roomDirectory.description')}</p>
        </div>
        <div className="room-directory__tools">
          <label className="room-directory__search">
            <Search aria-hidden="true" />
            <input
              aria-label={t('roomDirectory.search')}
              placeholder={t('roomDirectory.search')}
              type="search"
              value={search}
              onChange={(event) => {
                setSearch(event.target.value);
              }}
            />
          </label>
          <Button icon={<RefreshCw aria-hidden="true" />} onClick={onRefresh} tone="ghost">
            {t('roomDirectory.refresh')}
          </Button>
        </div>
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
          {visibleRooms.map((room, index) => (
            <motion.article
              animate={{ opacity: 1, y: 0 }}
              className="room-card"
              initial={reduceMotion ? false : { opacity: 0, y: 14 }}
              key={room.catalogId}
              transition={{ damping: 26, delay: index * 0.045, stiffness: 260, type: 'spring' }}
            >
              <div className="room-card__scene">
                <RoomIllustration />
              </div>
              <div className="room-card__body">
                <header>
                  <h2>{room.name}</h2>
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
              </div>
            </motion.article>
          ))}
          {visibleRooms.length === 0 ? (
            <DirectoryBoundary
              title={t('roomDirectory.noMatches')}
              detail={t('roomDirectory.noMatches.detail')}
              icon={<Search aria-hidden="true" />}
              role="status"
              action={
                <Button
                  onClick={() => {
                    setSearch('');
                  }}
                  tone="ghost"
                >
                  {t('roomDirectory.clear')}
                </Button>
              }
            />
          ) : null}
        </section>
      ) : null}
      <p className="room-directory__note">{t('roomDirectory.note')}</p>
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
