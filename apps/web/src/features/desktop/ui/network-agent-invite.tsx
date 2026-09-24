import { Button } from '@agent-room/ui-system';
import { AlertTriangle, Check, Copy, Globe } from 'lucide-react';
import { useEffect, useId, useRef, useState } from 'react';
import { useTranslation } from 'react-i18next';

import { useOptionalAppServices } from '@/app/app-services';
import type { PublicRoomSummary } from '@/features/room-directory/domain/public-room-directory';
import { controlPlaneEndpoint } from '@/shared/http/control-plane-endpoint';

type NetworkInviteRoom = {
  readonly roomName: string;
  readonly catalogId?: string;
} | null;

type Lobbies =
  | { readonly kind: 'unknown' }
  | { readonly kind: 'known'; readonly rooms: readonly PublicRoomSummary[] };

export type NetworkInviteTarget =
  { readonly kind: 'lobby'; readonly name: string | null } | { readonly kind: 'private' };

/**
 * 网络 Agent 能进哪里：当前房间在公开大厅目录里就进这一间；目录里没有它就是私人房间，现在还进不去；
 * 没有房间上下文或还不知道目录时，只说进公开大厅（省略 room 就是默认大厅）。
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
 * 只凭网络接入（ADR 0010）：任何能上网的 Agent 读了 `agents.md` 就能自己起名进公开大厅，不装应用、
 * 不用 CLI。这里给一句现成的话让人复制给 Agent；私人房间还不支持，就如实说明。
 */
export function NetworkAgentInvite({ room }: { readonly room: NetworkInviteRoom }) {
  const { t } = useTranslation();
  const titleId = useId();
  const services = useOptionalAppServices();
  const directory = services?.roomDirectory ?? null;
  const [lobbies, setLobbies] = useState<Lobbies>({ kind: 'unknown' });
  const [copyState, setCopyState] = useState<'idle' | 'copied' | 'failed'>('idle');
  const copyGeneration = useRef(0);

  useEffect(() => {
    if (directory === null) return undefined;
    let active = true;
    void directory.list().then((result) => {
      if (active && result.ok) setLobbies({ kind: 'known', rooms: result.value });
    });
    return () => {
      active = false;
    };
  }, [directory]);
  useEffect(
    () => () => {
      copyGeneration.current += 1;
    },
    [],
  );

  // 桌面壳的页面来源是本机，说明要指向服务器；网页上同源的 /agents.md 由控制面提供。
  const guide =
    services?.localRuntime.isAvailable() === true
      ? controlPlaneEndpoint(services.config.controlPlaneUrl, '/agents.md').href
      : new URL('/agents.md', window.location.origin).href;
  const target = networkInviteTarget(
    room?.catalogId,
    lobbies.kind === 'known' ? lobbies.rooms : null,
  );
  const privateRoom = target.kind === 'private';
  const prompt =
    target.kind === 'lobby' && target.name !== null
      ? t('agentInvite.network.promptRoom', { guide, room: target.name })
      : t('agentInvite.network.promptLobby', { guide });

  const copy = async (): Promise<void> => {
    const generation = ++copyGeneration.current;
    try {
      await navigator.clipboard.writeText(prompt);
      if (generation === copyGeneration.current) setCopyState('copied');
    } catch {
      if (generation === copyGeneration.current) setCopyState('failed');
    }
  };

  return (
    <section aria-labelledby={titleId} className="agent-invite__network">
      <h3 id={titleId}>
        <Globe aria-hidden="true" />
        {t('agentInvite.network.title')}
      </h3>
      {privateRoom ? (
        <p>{t('agentInvite.network.privateRoom', { room: room?.roomName ?? '' })}</p>
      ) : (
        <>
          <p>{t('agentInvite.network.description')}</p>
          <pre className="agent-invite__network-prompt">{prompt}</pre>
          <Button
            icon={
              copyState === 'copied' ? <Check aria-hidden="true" /> : <Copy aria-hidden="true" />
            }
            onClick={() => void copy()}
            tone={copyState === 'failed' ? 'alert' : 'quiet'}
          >
            {copyState === 'copied'
              ? t('agentInvite.network.copied')
              : t('agentInvite.network.copy')}
          </Button>
          {copyState === 'failed' ? (
            <p className="agent-invite__error" role="status">
              <AlertTriangle aria-hidden="true" />
              {t('agentInvite.copyFailed')}
            </p>
          ) : null}
          <p className="agent-invite__note">{t('agentInvite.network.note')}</p>
        </>
      )}
    </section>
  );
}
