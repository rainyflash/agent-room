import { useQuery } from '@tanstack/react-query';

import { useAppServices } from '@/app/app-services';
import { privateRoomListQueryOptions } from '@/features/private-rooms/data/private-room-queries';
import { allows, memberFor } from '@/features/private-rooms/domain/private-room';
import { PrivateRoomAgentKnockToasts } from '@/features/private-rooms/ui/private-room-agent-knocks';
import { useSession } from '@/features/session/ui/session-provider';

/** 房间页的地址是 `/lobby/<房间目录 ID>`；别的页不问有没有人敲门。 */
export function lobbyCatalogId(pathname: string): string | null {
  const match = /^\/lobby\/([^/]+)\/?$/u.exec(pathname);
  return match?.[1] === undefined ? null : decodeURIComponent(match[1]);
}

/**
 * 提示栈里的敲门（`specs/network-agents/knock.md`）：管理者开着使用中的私人房间时，有网络 Agent
 * 拿房间号敲门就挂一条，能直接让它进来。不是私人房间、不是管理者，就不问。
 */
export function AgentKnockNotice({ pathname }: { readonly pathname: string }) {
  const { privateRooms } = useAppServices();
  const { snapshot } = useSession();
  const principalId = snapshot.context.principal?.principalId;
  const catalogId = lobbyCatalogId(pathname);
  // 只在房间页上读私人房间列表（和房间页共用缓存）。
  const list = useQuery({
    ...privateRoomListQueryOptions(privateRooms, principalId),
    enabled: catalogId !== null && principalId !== undefined,
  });
  if (catalogId === null || principalId === undefined || list.data?.ok !== true) return null;
  const room = list.data.value.find(
    (candidate) => candidate.catalogId === catalogId && candidate.status === 'active',
  );
  if (room === undefined) return null;
  const manages =
    room.ownerPrincipalId === principalId || allows(memberFor(room, principalId), 'manage');
  return manages ? (
    <PrivateRoomAgentKnockToasts catalogId={catalogId} rooms={privateRooms} />
  ) : null;
}
