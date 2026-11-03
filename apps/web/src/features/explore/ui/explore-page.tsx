import { useQuery } from '@tanstack/react-query';
import { useState } from 'react';

import { useAppServices } from '@/app/app-services';
import { useExploreRooms } from '@/features/explore/data/explore-queries';
import type { ExploreFilter } from '@/features/explore/domain/explore';
import { publicRoomDirectoryQueryOptions } from '@/features/room-directory/data/public-room-directory-query';
import { NewRoomDialog } from '@/features/room-directory/ui/new-room-dialog';
import { useSession } from '@/features/session/ui/session-provider';

import { ExploreView, type ExploreAccount, type ExploreRoomsState } from './explore-view';

export type ExplorePageProps = {
  readonly filter: ExploreFilter;
  readonly onFilterChange: (filter: ExploreFilter) => void;
  /** 登录、注册完回到哪里：发现页本身，或者打开着的那间房的详情。 */
  readonly returnPath: string;
};

/**
 * 发现页连着服务。它在登录会话里（不像围观页），从顶栏点过来、点回去都不会断开消息；没登录的人打开时，
 * 会话先认定“没登录”、清掉登录状态，再去读目录，免得刚发出的请求被清理中止。
 */
export function ExplorePage({ filter, onFilterChange, returnPath }: ExplorePageProps) {
  const { config, controlPlane, explore, roomDirectory } = useAppServices();
  const { snapshot } = useSession();
  const { controlStatus, principal } = snapshot.context;
  const settled = controlStatus !== 'checking';
  const signedIn = controlStatus === 'ready' && principal !== null;
  const rooms = useExploreRooms(explore, filter, signedIn ? principal.principalId : null, settled);
  const lobbies = useQuery({ ...publicRoomDirectoryQueryOptions(roomDirectory), enabled: settled });
  const [creating, setCreating] = useState(false);
  const registrationOpen = config.registrationMode === 'open-email';
  const account: ExploreAccount = !settled
    ? { kind: 'checking' }
    : signedIn
      ? { kind: 'signed-in' }
      : {
          kind: 'signed-out',
          onRegister: () => {
            void controlPlane.beginAuthentication(returnPath, 'register');
          },
          onSignIn: () => {
            void controlPlane.beginAuthentication(returnPath, 'sign-in');
          },
          registrationOpen,
        };
  return (
    <>
      <ExploreView
        account={account}
        filter={filter}
        lobbies={lobbies.data?.ok === true ? lobbies.data.value : []}
        now={Date.now()}
        onCreatePublicRoom={
          signedIn
            ? () => {
                setCreating(true);
              }
            : undefined
        }
        onFilterChange={onFilterChange}
        onLoadMore={() => {
          void rooms.fetchNextPage();
        }}
        onRetry={() => {
          void rooms.refetch();
          void lobbies.refetch();
        }}
        rooms={roomsState(rooms)}
      />
      {creating ? (
        <NewRoomDialog
          initiallyListed
          onClose={() => {
            setCreating(false);
          }}
        />
      ) : null}
    </>
  );
}

function roomsState(query: ReturnType<typeof useExploreRooms>): ExploreRoomsState {
  const pages = query.data?.pages ?? [];
  const first = pages[0];
  if (first === undefined) return { kind: 'loading' };
  if (!first.ok) return { kind: 'failed', failure: first.error };
  const rooms = pages.flatMap((page) => (page.ok ? page.value.rooms : []));
  const more = query.isFetchingNextPage ? 'loading' : query.hasNextPage ? 'available' : 'none';
  return { kind: 'ready', more, rooms };
}
