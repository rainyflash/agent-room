import { Link } from '@tanstack/react-router';
import type { ReactNode } from 'react';

import type { LobbySearch } from '@/shared/routing/route-state';

export type SubRoomLinkProps = {
  readonly catalogId: string;
  /** 收件箱认得的这间的 Matrix 房间（`useCatalogUnread`）；不认得时走 `/lobby/<房间号>` 由入场页去找。 */
  readonly matrixRoomId: string | undefined;
  readonly children: ReactNode;
  readonly className?: string;
  readonly title?: string;
  readonly search?: LobbySearch;
  readonly onClick?: () => void;
  readonly 'aria-label'?: string;
  readonly 'aria-describedby'?: string | undefined;
  readonly 'data-unread'?: 'true' | undefined;
};

/** 打开一间房（子房间、父房间）：知道它的 Matrix 房间就直接打开，不然走入场页。 */
export function SubRoomLink({
  catalogId,
  children,
  matrixRoomId,
  search,
  ...rest
}: SubRoomLinkProps) {
  return matrixRoomId === undefined ? (
    <Link {...rest} params={{ catalogId }} search={search ?? {}} to="/lobby/$catalogId">
      {children}
    </Link>
  ) : (
    <Link
      {...rest}
      params={{ catalogId, roomId: matrixRoomId }}
      search={search ?? {}}
      to="/lobby/$catalogId/instance/$roomId"
    >
      {children}
    </Link>
  );
}
