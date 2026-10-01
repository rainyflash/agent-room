import { Banner, CopyBlock, Details, Spinner } from '@agent-room/ui-system';
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
      <>
        <PrivateRoomNetworkInvite
          catalogId={catalogId}
          guide={guide}
          onCopied={onCopied}
          roomName={room?.roomName ?? ''}
          rooms={services.privateRooms}
        />
        <ChatPageAgentHint guide={guide} />
      </>
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
      <ChatPageAgentHint guide={guide} />
    </>
  );
}

/**
 * 网页里只能聊天的助手自己发不了网络请求，读了说明也进不来；只有它所在的应用能加 MCP 连接器时，
 * 由主人把服务器的 `/mcp` 加进去才行。不点名具体应用，只说清楚条件和地址。
 */
function ChatPageAgentHint({ guide }: { readonly guide: string }) {
  const { t } = useTranslation();
  const mcp = useNetworkAgentMcpUrl();
  return (
    <Details
      className="agent-invite__chat-page"
      summary={t('agentInvite.network.chatPage.summary')}
    >
      <p className="agent-invite__note">{t('agentInvite.network.chatPage.detail')}</p>
      {mcp === null ? (
        <p className="agent-invite__note">{t('agentInvite.network.chatPage.inGuide', { guide })}</p>
      ) : (
        <CopyBlock
          copiedLabel={t('agentInvite.network.chatPage.copied')}
          copyLabel={t('agentInvite.network.chatPage.copy')}
          failedLabel={t('agentInvite.message.failed')}
          text={mcp}
          textLabel={t('agentInvite.network.chatPage.addressLabel')}
          tone="ghost"
        />
      )}
    </Details>
  );
}

/**
 * 网络 Agent 的 MCP 地址。桌面端连的就是服务器的 API 域名，地址和说明里写的一样；网页端经同源代理
 * 连服务器，不知道 API 域名，返回 null，改让人去说明开头找。
 */
export function useNetworkAgentMcpUrl(): string | null {
  const services = useOptionalAppServices();
  return services?.localRuntime.isAvailable() === true
    ? controlPlaneEndpoint(services.config.controlPlaneUrl, '/mcp').href
    : null;
}

/** 网络 Agent 说明的地址。桌面壳的页面来源是本机，要指向服务器；网页上同源的 /agents.md 由控制面提供。 */
export function useNetworkAgentGuideUrl(): string {
  const services = useOptionalAppServices();
  return services?.localRuntime.isAvailable() === true
    ? controlPlaneEndpoint(services.config.controlPlaneUrl, '/agents.md').href
    : new URL('/agents.md', window.location.origin).href;
}
