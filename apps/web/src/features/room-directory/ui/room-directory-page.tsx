import { Link } from '@tanstack/react-router';
import { ArrowRight, Telescope } from 'lucide-react';
import { useId, type ReactNode } from 'react';
import { useTranslation } from 'react-i18next';

import { AppNavigation } from '@/shared/ui/app-navigation';

import './room-directory-page.css';
import { MyRoomsSection } from './my-rooms-section';
import { RoomActions } from './room-actions';

/**
 * “房间”页只列你在的房间：你建的、加入的、收到的邀请（`specs/public-rooms/design.md`“别的页跟着改”）。
 * 公开房间和公共大厅挪到发现页，页面底下给一条去那里的路。
 */
export function RoomDirectoryPage() {
  return (
    <RoomDirectoryView
      myRooms={<MyRoomsSection />}
      roomActions={<RoomActions showSwitch={false} />}
    />
  );
}

export type RoomDirectoryViewProps = {
  readonly roomActions?: ReactNode;
  readonly myRooms?: ReactNode;
};

export function RoomDirectoryView({ roomActions, myRooms }: RoomDirectoryViewProps) {
  const { t } = useTranslation();
  return (
    <main className="room-directory" id="main-content">
      <AppNavigation active="rooms" />

      <section className="room-directory__hero">
        <div>
          <h1>{t('roomDirectory.title')}</h1>
          <p>{t('roomDirectory.description')}</p>
        </div>
        <div className="room-directory__tools">{roomActions}</div>
      </section>

      {myRooms}

      <ExploreInvitation />
      <p className="room-directory__note">{t('roomDirectory.note')}</p>
    </main>
  );
}

/** 去发现页找更多：公开房间和公共大厅都在那里。 */
function ExploreInvitation() {
  const { t } = useTranslation();
  const headingId = useId();
  return (
    <section aria-labelledby={headingId} className="room-directory__explore">
      <span aria-hidden="true" className="room-directory__explore-icon">
        <Telescope />
      </span>
      <div>
        <h2 id={headingId}>{t('roomDirectory.explore.title')}</h2>
        <p>{t('roomDirectory.explore.detail')}</p>
      </div>
      <Link className="ar-button ar-button--default ar-button--primary" search={{}} to="/explore">
        {t('roomDirectory.explore.go')}
        <ArrowRight aria-hidden="true" />
      </Link>
    </section>
  );
}
