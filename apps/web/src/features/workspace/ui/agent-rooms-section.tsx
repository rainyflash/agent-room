import { Button, Spinner } from '@agent-room/ui-system';
import { useQueryClient } from '@tanstack/react-query';
import { Globe2, LockKeyhole, LogIn } from 'lucide-react';
import { useRef, useState, type ReactNode } from 'react';
import { useTranslation } from 'react-i18next';

import { useAppServices } from '@/app/app-services';
import { usePublicRoomDirectory } from '@/features/room-directory/data/public-room-directory-query';
import { useMyPrivateRooms } from '@/features/room-directory/ui/use-rooms';
import { agentRoomsQueryKey, useAgentRooms } from '@/features/workspace/data/agent-rooms-query';
import type { AgentRoom } from '@/features/workspace/domain/agent-directory';
import { isNetworkInstance, liveInstances } from '@/features/workspace/domain/agent-card';
import type { FleetAgent } from '@/features/workspace/domain/agent-fleet';
import {
  AGENT_ROOM_BUSY,
  bringCandidates,
  bringWhenReady,
  type BringCandidate,
} from '@/features/workspace/domain/agent-rooms';

/** 带进、离开以后给人看的一句话。 */
export type AgentRoomNotice =
  | { readonly kind: 'brought'; readonly room: string; readonly entered: number }
  | { readonly kind: 'left'; readonly room: string; readonly left: boolean }
  | { readonly kind: 'failed'; readonly room: string; readonly code: string };

const FORBIDDEN = 'lobby.unauthorized';

const sleep = async (milliseconds: number): Promise<void> => {
  await new Promise((resolve) => globalThis.setTimeout(resolve, milliseconds));
};

/**
 * Agent 详情里的“在哪些房间”（`specs/agent-identity/design.md`“网页”）：每个房间一行，带“离开”；
 * “带进房间”列出你能带它进的房间。服务器替你办，不用 Agent 正在线。网络 Agent 只看不动。
 */
export function AgentRoomsSection({ agent }: { readonly agent: FleetAgent }) {
  const { agentDirectory } = useAppServices();
  const queryClient = useQueryClient();
  const agentId = agent.agent.agentId;
  const rooms = useAgentRooms(agentDirectory, agentId);
  const [picking, setPicking] = useState(false);
  const [pendingCatalogId, setPendingCatalogId] = useState<string | null>(null);
  const pendingRef = useRef(false);
  const [notice, setNotice] = useState<AgentRoomNotice | null>(null);

  const run = async (
    room: { readonly catalogId: string; readonly name: string },
    action: () => Promise<AgentRoomNotice>,
  ): Promise<void> => {
    if (pendingRef.current) return;
    pendingRef.current = true;
    setPendingCatalogId(room.catalogId);
    setNotice(null);
    try {
      setNotice(await action());
    } finally {
      pendingRef.current = false;
      setPendingCatalogId(null);
      await queryClient.invalidateQueries({ queryKey: [...agentRoomsQueryKey, agentId] });
    }
  };

  const bring = (room: BringCandidate) =>
    void run(room, async () => {
      const result = await bringWhenReady(
        async () => await agentDirectory.bringAgentIntoRoom(agentId, room.catalogId),
        sleep,
      );
      if (!result.ok) return { kind: 'failed', room: room.name, code: result.error.code };
      setPicking(false);
      return { kind: 'brought', room: room.name, entered: result.value };
    });

  const leave = (room: AgentRoom) =>
    void run(room, async () => {
      const result = await agentDirectory.leaveAgentRoom(agentId, room.catalogId);
      return result.ok
        ? { kind: 'left', room: room.name, left: result.value }
        : { kind: 'failed', room: room.name, code: result.error.code };
    });

  const current = rooms.data?.ok === true ? rooms.data.value : [];
  return (
    <AgentRoomsView
      agentName={agent.agent.displayName}
      managed={!liveInstances(agent).some(isNetworkInstance)}
      notice={notice}
      onLeave={leave}
      onPick={setPicking}
      onRetry={() => void rooms.refetch()}
      pendingCatalogId={pendingCatalogId}
      picker={
        picking ? (
          <BringPicker current={current} onBring={bring} pendingCatalogId={pendingCatalogId} />
        ) : null
      }
      rooms={current}
      state={rooms.isPending ? 'loading' : rooms.data?.ok === true ? 'ready' : 'failed'}
    />
  );
}

/** 房间选择要读你的私人房间和公共大厅，打开时才去读。 */
function BringPicker({
  current,
  onBring,
  pendingCatalogId,
}: {
  readonly current: readonly AgentRoom[];
  readonly onBring: (room: BringCandidate) => void;
  readonly pendingCatalogId: string | null;
}) {
  const { roomDirectory } = useAppServices();
  const mine = useMyPrivateRooms();
  const directory = usePublicRoomDirectory(roomDirectory);
  const publicRooms = directory.data?.ok === true ? directory.data.value : null;
  const state =
    mine.state === 'failed' || (!directory.isPending && publicRooms === null)
      ? 'failed'
      : mine.state === 'loading' || directory.isPending
        ? 'loading'
        : 'ready';
  return (
    <BringCandidates
      candidates={bringCandidates({
        current,
        principalId: mine.principalId,
        privateRooms: mine.joined,
        publicRooms: publicRooms ?? [],
      })}
      onBring={onBring}
      pendingCatalogId={pendingCatalogId}
      state={state}
    />
  );
}

export type AgentRoomsViewProps = {
  readonly agentName: string;
  /** 本机的 Agent 能带进、离开；网络 Agent 自己决定进出，只看不动。 */
  readonly managed: boolean;
  readonly notice: AgentRoomNotice | null;
  readonly onLeave: (room: AgentRoom) => void;
  readonly onPick: (picking: boolean) => void;
  readonly onRetry: () => void;
  /** 正在带进或离开的房间；这时别的操作先停着。 */
  readonly pendingCatalogId: string | null;
  /** 打开着的房间选择；`null` 时显示“带进房间”按钮。 */
  readonly picker: ReactNode;
  readonly rooms: readonly AgentRoom[];
  readonly state: 'failed' | 'loading' | 'ready';
};

export function AgentRoomsView({
  agentName,
  managed,
  notice,
  onLeave,
  onPick,
  onRetry,
  pendingCatalogId,
  picker,
  rooms,
  state,
}: AgentRoomsViewProps) {
  const { t } = useTranslation();
  const busy = pendingCatalogId !== null;
  return (
    <section aria-labelledby="agent-details-rooms" className="agent-details__rooms">
      <h3 id="agent-details-rooms">{t('workspace.rooms.title')}</h3>
      {state === 'loading' ? (
        <p className="agent-details__muted" role="status">
          <Spinner />
          {t('workspace.rooms.loading')}
        </p>
      ) : state === 'failed' ? (
        <p className="agent-details__muted">
          {t('workspace.rooms.failed')}{' '}
          <Button onClick={onRetry} size="compact" tone="quiet">
            {t('workspace.rooms.retry')}
          </Button>
        </p>
      ) : rooms.length === 0 ? (
        <p className="agent-details__muted">{t('workspace.rooms.none')}</p>
      ) : (
        <ul>
          {rooms.map((room) => (
            <li key={room.catalogId}>
              <RoomIcon kind={room.kind} />
              <strong>{room.name}</strong>
              <span className="agent-details__muted">
                {t(
                  room.kind === 'public_lobby'
                    ? 'workspace.rooms.public'
                    : 'workspace.rooms.private',
                )}
              </span>
              {room.archived ? (
                <span className="agent-details__tag">{t('workspace.rooms.archived')}</span>
              ) : null}
              {managed ? (
                <Button
                  aria-label={t('workspace.rooms.leaveLabel', {
                    agent: agentName,
                    room: room.name,
                  })}
                  className="agent-details__room-action"
                  disabled={busy}
                  icon={pendingCatalogId === room.catalogId ? <Spinner /> : undefined}
                  onClick={() => {
                    onLeave(room);
                  }}
                  size="compact"
                  tone="quiet"
                >
                  {t('workspace.rooms.leave')}
                </Button>
              ) : null}
            </li>
          ))}
        </ul>
      )}
      {notice === null ? null : <RoomNotice notice={notice} />}
      {!managed ? (
        <p className="agent-details__muted">{t('workspace.rooms.network')}</p>
      ) : picker === null ? (
        <div>
          <Button
            disabled={busy}
            icon={<LogIn aria-hidden="true" />}
            onClick={() => {
              onPick(true);
            }}
            size="compact"
          >
            {t('workspace.rooms.bring')}
          </Button>
        </div>
      ) : (
        <div className="agent-details__picker">
          {picker}
          <div>
            <Button
              disabled={busy}
              onClick={() => {
                onPick(false);
              }}
              size="compact"
              tone="quiet"
            >
              {t('workspace.rooms.cancel')}
            </Button>
          </div>
        </div>
      )}
    </section>
  );
}

export function BringCandidates({
  candidates,
  onBring,
  pendingCatalogId = null,
  state,
}: {
  readonly candidates: readonly BringCandidate[];
  readonly onBring: (room: BringCandidate) => void;
  readonly pendingCatalogId?: string | null;
  readonly state: 'failed' | 'loading' | 'ready';
}) {
  const { t } = useTranslation();
  return (
    <div aria-labelledby="agent-details-pick" className="agent-details__candidates" role="group">
      <p>
        <strong id="agent-details-pick">{t('workspace.rooms.pick')}</strong>{' '}
        {t('workspace.rooms.pickDetail')}
      </p>
      {state === 'loading' ? (
        <p className="agent-details__muted" role="status">
          <Spinner />
          {t('workspace.rooms.pickLoading')}
        </p>
      ) : state === 'failed' ? (
        <p className="agent-details__muted">{t('workspace.rooms.pickFailed')}</p>
      ) : candidates.length === 0 ? (
        <p className="agent-details__muted">{t('workspace.rooms.pickEmpty')}</p>
      ) : (
        <ul>
          {candidates.map((room) => (
            <li key={room.catalogId}>
              <button
                className="agent-details__candidate"
                disabled={pendingCatalogId !== null}
                onClick={() => {
                  onBring(room);
                }}
                type="button"
              >
                <RoomIcon kind={room.kind} />
                <span>
                  <strong>{room.name}</strong>
                  <small>
                    {t(
                      room.kind === 'public_lobby'
                        ? 'workspace.rooms.public'
                        : 'workspace.rooms.private',
                    )}
                  </small>
                  {room.viewOnly ? (
                    <small className="agent-details__view-only">
                      {t('workspace.rooms.viewOnly')}
                    </small>
                  ) : null}
                </span>
                {pendingCatalogId === room.catalogId ? <Spinner /> : null}
              </button>
            </li>
          ))}
        </ul>
      )}
    </div>
  );
}

function RoomIcon({ kind }: { readonly kind: AgentRoom['kind'] }) {
  return kind === 'public_lobby' ? (
    <Globe2 aria-hidden="true" className="agent-details__room-icon" />
  ) : (
    <LockKeyhole aria-hidden="true" className="agent-details__room-icon" />
  );
}

function RoomNotice({ notice }: { readonly notice: AgentRoomNotice }) {
  const { t } = useTranslation();
  switch (notice.kind) {
    case 'brought':
      return (
        <p className="agent-details__notice" role="status">
          {notice.entered === 0
            ? t('workspace.rooms.noConnection')
            : t('workspace.rooms.brought', { room: notice.room })}
        </p>
      );
    case 'left':
      return (
        <p className="agent-details__notice" role="status">
          {t(notice.left ? 'workspace.rooms.left' : 'workspace.rooms.notInside', {
            room: notice.room,
          })}
        </p>
      );
    case 'failed':
      return (
        <p className="agent-details__notice" role="alert">
          {notice.code === AGENT_ROOM_BUSY
            ? t('workspace.rooms.busy', { room: notice.room })
            : notice.code === FORBIDDEN
              ? t('workspace.rooms.forbidden')
              : t('workspace.rooms.actionFailed', { code: notice.code })}
        </p>
      );
  }
}
