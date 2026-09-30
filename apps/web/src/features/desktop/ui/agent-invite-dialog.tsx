import { Dialog, Segmented } from '@agent-room/ui-system';
import { Bot, Globe, Plug, SquareTerminal } from 'lucide-react';
import { useEffect, useRef, useState } from 'react';
import { useTranslation } from 'react-i18next';

import { useOptionalAppServices } from '@/app/app-services';
import { useOptionalSession } from '@/features/session/ui/session-provider';
import { BrowserUuidV7Factory } from '@/shared/ids/browser-uuid-v7-factory';
import {
  hostSessionArrivals,
  mergeArrivals,
  rosterArrivals,
  type AgentArrival,
  type PresentAgent,
} from '../domain/agent-arrivals';
import { localConnectionReady } from '../domain/desktop-connection';
import type { HostSessionDiagnostics, InvitationOffer } from '../domain/desktop-runtime';
import type { ConnectedInvitation } from '../domain/invite-reply';
import { AgentArrivalsPanel } from './agent-arrivals-panel';
import { useDesktopRuntimeController } from './desktop-runtime-provider';
import { LocalAgentInvite } from './local-agent-invite';
import { NetworkAgentInvite } from './network-agent-invite';
import './agent-invite-dialog.css';

export type InviteRoom = {
  readonly roomId: string;
  readonly roomName: string;
  readonly catalogId?: string;
} | null;
type InviteOwner = { readonly principalId: string; readonly displayName: string } | null;

export type AgentInviteDialogProps = {
  /** 当前所在房间；没有房间时（从“我的 Agent”打开）Agent 进公共大厅。 */
  readonly room: InviteRoom;
  /** 已登录用户；缺省时从会话上下文读取。 */
  readonly owner?: InviteOwner;
  readonly downloadUrl: string | null;
  /** 房间里已经在场的 Agent：之后新出现的算作刚进来的。 */
  readonly presentAgents?: readonly PresentAgent[];
  readonly onClose: () => void;
  readonly onStartConversation?: (() => void) | undefined;
  readonly onConnected?: ((invitation: ConnectedInvitation) => void) | undefined;
};

const inviteMethods = ['network', 'mcp', 'cli'] as const;
export type InviteMethod = (typeof inviteMethods)[number];
const inviteMethodKey = 'agent-room.invite-method';
const SESSION_POLL_MS = 3_000;
/** Bridge 十分钟不续期就撤掉挂着的人物；对话框开着时提前续上。 */
const OFFER_REFRESH_MS = 3 * 60_000;
const uuid = new BrowserUuidV7Factory();
const noAgents: readonly PresentAgent[] = [];

/** 上次选的接入方式存在本机；读不到（隐私模式等）就从最通用的网络接入开始。 */
function readInviteMethod(storage: Storage | null): InviteMethod {
  try {
    const saved = storage?.getItem(inviteMethodKey);
    return inviteMethods.find((method) => method === saved) ?? 'network';
  } catch {
    return 'network';
  }
}

function saveInviteMethod(storage: Storage | null, method: InviteMethod): void {
  try {
    storage?.setItem(inviteMethodKey, method);
  } catch {
    // 只在这次有效。
  }
}

function localStorageOrNull(): Storage | null {
  try {
    return window.localStorage;
  } catch {
    return null;
  }
}

/**
 * 接入 Agent：选一种方式，复制一段话发给 Agent，它进来时就显示在下面。三种方式并列，不按具体的
 * Agent 应用区分；Agent 自己起名，同一个任务再接入会回到同一个人物，所以这里不再管人物和名字。
 */
export function AgentInviteDialog({
  room,
  owner = null,
  downloadUrl,
  presentAgents = noAgents,
  onClose,
  onStartConversation,
  onConnected,
}: AgentInviteDialogProps) {
  const { t } = useTranslation();
  return (
    <Dialog
      className="agent-invite"
      closeLabel={t('agentInvite.close')}
      description={
        room === null
          ? t('agentInvite.subtitle.lobby')
          : t('agentInvite.subtitle.room', { room: room.roomName })
      }
      icon={<Bot />}
      onClose={onClose}
      title={t('agentInvite.title')}
    >
      <InviteBody
        downloadUrl={downloadUrl}
        onClose={onClose}
        onConnected={onConnected}
        onStartConversation={onStartConversation}
        owner={owner}
        presentAgents={presentAgents}
        room={room}
      />
    </Dialog>
  );
}

function InviteBody({
  room,
  owner: ownerProp,
  downloadUrl,
  presentAgents,
  onClose,
  onStartConversation,
  onConnected,
}: Required<Omit<AgentInviteDialogProps, 'owner'>> & { readonly owner: InviteOwner }) {
  const { t } = useTranslation();
  const services = useOptionalAppServices();
  const session = useOptionalSession();
  const principal = session?.snapshot.context.principal ?? null;
  const owner =
    ownerProp ??
    (principal === null
      ? null
      : { principalId: principal.principalId, displayName: principal.displayName });
  const [storage] = useState(localStorageOrNull);
  const [method, setMethod] = useState<InviteMethod>(() => readInviteMethod(storage));
  const [openedAt] = useState(() => Date.now());
  const [copiedAt, setCopiedAt] = useState<number | null>(null);
  const local = method !== 'network';
  const { sessions, sessionsFailure, localReady, desktop } = useHostSessions(local);
  const [hostBaseline, setHostBaseline] = useState<ReadonlySet<string> | null>(null);
  // 第一次读到本机会话时记下已有的；之后多出来的才算刚进来的。
  if (hostBaseline === null && sessions !== null) {
    setHostBaseline(new Set(sessions.map((entry) => entry.session.sessionId)));
  }
  const [rosterBaseline] = useState(() => new Set(presentAgents.map((agent) => agent.agentId)));
  const arrivals = mergeArrivals(
    sessions === null || hostBaseline === null
      ? []
      : hostSessionArrivals(sessions, hostBaseline, room),
    rosterArrivals(presentAgents, rosterBaseline),
  );
  useParkedInvitation(local && localReady, room, sessions);
  useConnectedCallback(arrivals, room, copiedAt ?? openedAt, onConnected);

  const chooseMethod = (next: InviteMethod) => {
    if (next === method) return;
    setMethod(next);
    saveInviteMethod(storage, next);
    setCopiedAt(null);
  };
  const copied = () => {
    setCopiedAt(Date.now());
  };

  return (
    <>
      <Segmented
        label={t('agentInvite.mode')}
        onChange={chooseMethod}
        options={[
          { value: 'network', label: t('agentInvite.method.network'), icon: <Globe /> },
          { value: 'mcp', label: t('agentInvite.method.mcp'), icon: <Plug /> },
          { value: 'cli', label: t('agentInvite.method.cli'), icon: <SquareTerminal /> },
        ]}
        value={method}
      />
      <p className="agent-invite__lead">{t(`agentInvite.method.${method}Hint`)}</p>
      {method === 'network' ? (
        <NetworkAgentInvite onCopied={copied} room={room} />
      ) : (
        <LocalAgentInvite downloadUrl={downloadUrl} method={method} onCopied={copied} room={room} />
      )}
      <AgentArrivalsPanel
        arrivals={arrivals}
        copiedAt={copiedAt}
        observable={room !== null || local}
        onDone={onStartConversation ?? onClose}
        startConversation={onStartConversation !== undefined}
        firstReply={
          services === null || room === null || owner === null
            ? null
            : {
                gateway: services.messages,
                principalId: owner.principalId,
                roomId: room.roomId,
                startedAt: copiedAt ?? openedAt,
              }
        }
        unavailable={local && sessionsFailure !== null}
      />
      {local && desktop ? (
        <p className="agent-invite__note">{t('agentInvite.backgroundHint')}</p>
      ) : null}
    </>
  );
}

/** 本机方式时每 3 秒读一次这台电脑上的 Agent 会话；网络方式和网页端不读。 */
function useHostSessions(local: boolean): {
  readonly sessions: readonly HostSessionDiagnostics[] | null;
  readonly sessionsFailure: string | null;
  readonly localReady: boolean;
  readonly desktop: boolean;
} {
  const controller = useDesktopRuntimeController();
  const phase = controller.snapshot?.bridge.lifecycle.phase ?? 'discovering';
  const localReady = controller.available && localConnectionReady(phase);
  const { readHostSessions } = controller;
  const [sessions, setSessions] = useState<readonly HostSessionDiagnostics[] | null>(null);
  const [sessionsFailure, setSessionsFailure] = useState<string | null>(null);
  useEffect(() => {
    if (!localReady || !local) return undefined;
    let disposed = false;
    let timer: ReturnType<typeof setTimeout> | undefined;
    const poll = async () => {
      const result = await readHostSessions();
      if (disposed) return;
      if (result.ok) {
        setSessions(result.value);
        setSessionsFailure(null);
      } else setSessionsFailure(result.error.code);
      timer = setTimeout(() => void poll(), SESSION_POLL_MS);
    };
    void poll();
    return () => {
      disposed = true;
      clearTimeout(timer);
    };
  }, [localReady, local, readHostSessions]);
  return { sessions, sessionsFailure, localReady, desktop: controller.available };
}

/**
 * 本机方式时把一个人物挂在这台电脑的连接服务上：已经配好 MCP 的 Agent 只要听到“接入 Agent Room”
 * 就能直接进到这个房间，不用复制。它接上后不再续期，对话框关掉或换成网络方式时撤回。
 */
function useParkedInvitation(
  active: boolean,
  room: InviteRoom,
  sessions: readonly HostSessionDiagnostics[] | null,
): void {
  const controller = useDesktopRuntimeController();
  const { offerInvitation, withdrawInvitation } = controller;
  const [sessionKey] = useState(() => uuid.next());
  const taken = sessions?.some((entry) => entry.sessionKey === sessionKey) ?? false;
  const catalogId = room?.catalogId;
  const roomId = room?.roomId;
  useEffect(() => {
    if (!active || taken) return undefined;
    const invitation: InvitationOffer = {
      sessionKey,
      ...(catalogId === undefined
        ? {}
        : { room: { catalogId, ...(roomId === undefined ? {} : { roomId }) } }),
    };
    let disposed = false;
    let timer: ReturnType<typeof setTimeout> | undefined;
    const offer = async () => {
      await offerInvitation(invitation);
      if (disposed) return;
      timer = setTimeout(() => void offer(), OFFER_REFRESH_MS);
    };
    void offer();
    return () => {
      disposed = true;
      clearTimeout(timer);
      void withdrawInvitation(sessionKey);
    };
  }, [active, taken, sessionKey, catalogId, roomId, offerInvitation, withdrawInvitation]);
}

/** 第一个进了这个房间的 Agent：告诉房间页，它好在关掉对话框后接着显示首次回复的进度。 */
function useConnectedCallback(
  arrivals: readonly AgentArrival[],
  room: InviteRoom,
  startedAt: number,
  onConnected: ((invitation: ConnectedInvitation) => void) | undefined,
) {
  const reported = useRef<string | null>(null);
  const first = arrivals.find(
    (arrival) => arrival.state === 'ready' && !arrival.elsewhere && arrival.agentId !== null,
  );
  const agentId = first?.agentId ?? null;
  const displayName = first?.displayName ?? null;
  const roomId = room?.roomId ?? null;
  useEffect(() => {
    if (agentId === null || displayName === null || roomId === null) return;
    if (reported.current === agentId) return;
    reported.current = agentId;
    onConnected?.({ agentId, roomId, startedAt, displayName });
  }, [agentId, displayName, roomId, startedAt, onConnected]);
}
