import { Banner, CopyBlock, Spinner } from '@agent-room/ui-system';
import { useEffect, useState } from 'react';
import { useTranslation } from 'react-i18next';

import { useOptionalAppServices } from '@/app/app-services';
import { PrivateRoomNetworkInvite } from '@/features/private-rooms/ui/private-room-network-invite';
import type { PublicRoomSummary } from '@/features/room-directory/domain/public-room-directory';
import { controlPlaneEndpoint } from '@/shared/http/control-plane-endpoint';

type NetworkInviteRoom = {
  readonly roomName: string;
  readonly catalogId?: string;
} | null;

type Lobbies =
  | { readonly kind: 'unknown' }
  | { readonly kind: 'failed' }
  | { readonly kind: 'known'; readonly rooms: readonly PublicRoomSummary[] };

export type NetworkInviteTarget =
  { readonly kind: 'lobby'; readonly name: string | null } | { readonly kind: 'private' };

/**
 * 网络 Agent 能进哪里：当前房间在公共大厅目录里就进这一间；目录里没有它就是私人房间，要凭口令进；
 * 没有房间时，或者读不到目录时，只说进公共大厅（不写房间名就是默认大厅）。
 */
export function networkInviteTarget(
  catalogId: string | undefined,
  lobbies: readonly PublicRoomSummary[] | null,
): NetworkInviteTarget {
  if (catalogId === undefined || lobbies === null) return { kind: 'lobby', name: null };
  const lobby = lobbies.find((entry) => entry.catalogId === catalogId);
  return lobby === undefined ? { kind: 'private' } : { kind: 'lobby', name: lobby.name };
}

/**
 * 只凭网络接入（ADR 0010）：任何能上网的 Agent 读了 `agents.md` 就能自己起名进来，不装应用、
 * 不用命令行。给一句现成的话复制给 Agent；私人房间要口令，就在这里一键生成并复制。
 */
export function NetworkAgentInvite({
  room,
  onCopied,
}: {
  readonly room: NetworkInviteRoom;
  readonly onCopied?: () => void;
}) {
  const { t } = useTranslation();
  const services = useOptionalAppServices();
  const directory = services?.roomDirectory ?? null;
  const [lobbies, setLobbies] = useState<Lobbies>({ kind: 'unknown' });
  const catalogId = room?.catalogId;

  // 只有知道当前房间时才需要目录：判断它是不是公共大厅。
  useEffect(() => {
    if (directory === null || catalogId === undefined) return undefined;
    let active = true;
    void directory.list().then((result) => {
      if (!active) return;
      setLobbies(result.ok ? { kind: 'known', rooms: result.value } : { kind: 'failed' });
    });
    return () => {
      active = false;
    };
  }, [directory, catalogId]);

  const guide = useNetworkAgentGuideUrl();
  // 读目录的这一小会儿还不知道是不是私人房间：先别给出进公共大厅的那句话。
  if (directory !== null && catalogId !== undefined && lobbies.kind === 'unknown') {
    return (
      <p className="agent-invite__waiting">
        <Spinner />
        {t('agentInvite.network.checking')}
      </p>
    );
  }
  const target = networkInviteTarget(catalogId, lobbies.kind === 'known' ? lobbies.rooms : null);
  if (target.kind === 'private') {
    return services === null || catalogId === undefined ? (
      <Banner role={null} tone="info">
        {t('agentInvite.network.privateRoom', { room: room?.roomName ?? '' })}
      </Banner>
    ) : (
      <PrivateRoomNetworkInvite
        catalogId={catalogId}
        guide={guide}
        onCopied={onCopied}
        roomName={room?.roomName ?? ''}
        rooms={services.privateRooms}
      />
    );
  }
  const prompt =
    target.name === null
      ? t('agentInvite.network.promptLobby', { guide })
      : t('agentInvite.network.promptRoom', { guide, room: target.name });
  return (
    <>
      {lobbies.kind === 'failed' ? (
        <p className="agent-invite__note">{t('agentInvite.network.checkFailed')}</p>
      ) : null}
      <CopyBlock
        copiedLabel={t('agentInvite.message.copied')}
        copyLabel={t('agentInvite.message.copy')}
        failedLabel={t('agentInvite.message.failed')}
        onCopied={onCopied}
        text={prompt}
        textLabel={t('agentInvite.message.label')}
      />
      <p className="agent-invite__note">{t('agentInvite.network.note')}</p>
    </>
  );
}

/** 网络 Agent 说明的地址。桌面壳的页面来源是本机，要指向服务器；网页上同源的 /agents.md 由控制面提供。 */
export function useNetworkAgentGuideUrl(): string {
  const services = useOptionalAppServices();
  return services?.localRuntime.isAvailable() === true
    ? controlPlaneEndpoint(services.config.controlPlaneUrl, '/agents.md').href
    : new URL('/agents.md', window.location.origin).href;
}
