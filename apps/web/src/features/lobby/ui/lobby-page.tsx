import { usePublishedDownload } from '@/features/updates/ui/use-published-download';
import {
  Archive,
  Bot,
  Files,
  Maximize2,
  MessageCircle,
  MessageSquareOff,
  Minimize2,
  Share2,
  UserPlus,
  UsersRound,
  X,
} from 'lucide-react';
import { AnimatePresence } from 'motion/react';
import { useEffect, useMemo, useRef, useState, useSyncExternalStore } from 'react';
import { useTranslation } from 'react-i18next';
import { useAppServices } from '@/app/app-services';
import { ConversationSearchRequestContext } from '@/features/conversation/ui/conversation-search-request';
import { ConversationWorkspaceProvider } from '@/features/conversation/ui/conversation-workspace-context';
import { AgentInviteDialog } from '@/features/desktop/ui/agent-invite-dialog';
import { useLocalAgentIds } from '@/features/desktop/ui/use-local-agent-ids';
import { ReceptionPanel } from '@/features/desktop/ui/reception-panel';
import { ReceptionOwnershipPanel } from '@/features/desktop/ui/reception-ownership-panel';
import { InviteReplyProgress } from '@/features/desktop/ui/invite-reply-progress';
import type { ConnectedInvitation } from '@/features/desktop/domain/invite-reply';
import {
  ReceptionEvidenceProvider,
  useReceptionEvidence,
} from '@/features/desktop/ui/reception-evidence-context';
import { hasAutomationDraftFor } from '@/features/automation/ui/automation-settings';
import { DirectConversationDock } from '@/features/direct-sessions/ui/direct-conversation-dock';
import {
  useDirectSessionController,
  type DirectSessionController,
} from '@/features/direct-sessions/ui/use-direct-session-controller';
import { LobbyExperienceStore } from '@/features/lobby/application/lobby-experience-store';
import { RoomActivityStore } from '@/features/lobby/application/room-activity-store';
import { useUnreadTitle } from '@/shared/browser/use-unread-title';
import { onWindowForegroundChange, windowInForeground } from '@/shared/browser/window-foreground';
import {
  RoomMessagesProvider,
  useRoomMessages,
} from '@/features/messages/ui/room-messages-context';
import { projectRoomSpeech } from '@/features/lobby/domain/room-speech';
import type { LobbyRoom } from '@/features/lobby/domain/lobby';
import { attendanceCounts } from '@/features/lobby/domain/agent-attendance';
import type {
  RoomPageLayout,
  RoomPageView,
  RoomPanelView,
  RoomWorkspaceView,
} from '@/features/lobby/domain/workspace-view';
import type { LobbySceneProjection } from '@/features/lobby/domain/scene-projection';
import { AgentInspector, type AgentInspectorFailure } from '@/features/lobby/ui/agent-inspector';
import { AgentRosterPolicyPanel } from './agent-roster-policy-panel';
import { ListModeRoster } from '@/features/lobby/ui/list-mode-roster';
import { AgentTablePage } from '@/features/lobby/ui/agent-table-page';
import { RoomRoster } from '@/features/lobby/ui/room-roster';
import { useRoomRoster } from '@/features/lobby/ui/use-room-roster';
import { useMediaQuery } from '@/shared/ui/use-media-query';
import {
  RoomSettingsDialog,
  type RoomSettingsSection,
} from '@/features/lobby/ui/room-settings-dialog';
import {
  LobbySpatialView,
  type LobbySpatialViewHandle,
} from '@/features/lobby/ui/lobby-spatial-view';
import { LobbyStateBoundary } from '@/features/lobby/ui/lobby-state-boundary';
import { PrivateRoomJoinBoundary } from '@/features/lobby/ui/private-room-join-boundary';
import { MatrixConnectionCard } from '@/features/session/ui/matrix-connection-card';
import { AgentOwnersScope } from '@/features/lobby/ui/agent-owners';
import { NetworkAgentRelayProvider } from '@/features/lobby/ui/network-agent-labels';
import { allows, memberFor, type PrivateRoom } from '@/features/private-rooms/domain/private-room';
import { PrivateRoomInviteDialog } from '@/features/private-rooms/ui/private-room-invite-dialog';
import { RoomBriefs } from '@/features/room-notice/ui/room-briefs';
import {
  RoomPinsActionsContext,
  useRoomPinsActions,
} from '@/features/room-pins/ui/room-pins-actions';
import { RoomPinsBar } from '@/features/room-pins/ui/room-pins-bar';
import { usePendingJoinRequestCount } from '@/features/private-rooms/ui/join-request-attention';
import { RoomBeacon } from '@/features/lobby/ui/room-beacon';
import { RoomDesk, roomDeskSideQuery } from '@/features/lobby/ui/room-desk';
import { RoomLayoutSwitch } from '@/features/lobby/ui/room-layout-switch';
import { WorkspaceDrawer } from '@/features/lobby/ui/workspace-drawer';
import { DirectSessionNavigation } from '@/features/direct-sessions/ui/direct-session-navigation';
import { WorkspaceViewTabs } from '@/features/lobby/ui/workspace-view-tabs';
import { MessageLayer } from '@/features/messages/ui/message-layer';
import {
  usePrivateRoomListWith,
  usePrivateRoomListing,
  usePrivateRoomOrigin,
} from '@/features/private-rooms/data/private-room-queries';
import { useRoomRepositoryMark } from '@/features/private-rooms/data/project-room-queries';
import { useRoomLiveView } from '@/features/private-rooms/data/room-live-view-queries';
import { useSubRooms } from '@/features/private-rooms/data/sub-room-queries';
import type { SubRoom } from '@/features/private-rooms/domain/sub-rooms';
import { RepositoryBadgeShare } from '@/features/private-rooms/ui/project-room-badge';
import { isSubRoomOf } from '@/features/sub-rooms/domain/sub-room-places';
import { ArchivedSubRoomNotice } from '@/features/sub-rooms/ui/archived-sub-room-notice';
import { NewSubRoomDialog } from '@/features/sub-rooms/ui/new-sub-room-dialog';
import { SubRoomBrowser } from '@/features/sub-rooms/ui/sub-room-browser';
import { SubRoomDoor } from '@/features/sub-rooms/ui/sub-room-door';
import { SubRoomPlaces } from '@/features/sub-rooms/ui/sub-room-places';
import type { SubRoomRowMenuContext } from '@/features/sub-rooms/ui/sub-room-row-menu';
import { useUnseenSubRooms } from '@/features/sub-rooms/ui/use-unseen-sub-rooms';
import { shareable } from '@/features/private-rooms/domain/private-room-listing';
import { ShareRoomDialog } from '@/features/room-share/ui/share-room-dialog';
import { conversationMessages } from '@/features/conversation/domain/conversation';
import {
  ExcerptSelectionContext,
  useExcerptSharing,
} from '@/features/room-excerpts/ui/excerpt-selection';
import { ExcerptSelectionBar } from '@/features/room-excerpts/ui/excerpt-selection-bar';
import { ShareExcerptDialog } from '@/features/room-excerpts/ui/share-excerpt-dialog';
import { SharedExcerpts } from '@/features/room-excerpts/ui/shared-excerpts';
import type { ShareableRoom } from '@/features/room-share/domain/room-share';
import type { WebSession } from '@/features/session/domain/session';
import './lobby-workspace.css';
import './lobby-game.css';

// 对话面板放大与否只影响这台设备的布局，存在本机；读写失败（隐私模式等）就只在本次有效。
const panelSizeKey = 'agent-room.room-panel-size';
/** 还没有人说过话时用的同一份空名单：每次重画都新建一份的话，右栏的人会跟着重算。 */
const noHumans: readonly never[] = [];
/** 你管着哪间子房间时，多久再问一次子房间名单：左栏和齿轮上“有人在等批准”的点跟着它。 */
const SUB_ROOM_WAITING_POLL_MS = 30_000;
type PanelSize = 'regular' | 'wide';

function readPanelSize(): PanelSize {
  try {
    return window.localStorage.getItem(panelSizeKey) === 'wide' ? 'wide' : 'regular';
  } catch {
    return 'regular';
  }
}

function writePanelSize(size: PanelSize): void {
  try {
    window.localStorage.setItem(panelSizeKey, size);
  } catch {
    // 只在本次有效。
  }
}

export type LobbyPageProps = {
  readonly catalogId: string;
  /** 地址里写着的、你还没进的一间子房间：在这一页上展开成它的卡片。 */
  readonly door: string | null;
  /** 对话占主栏，还是原来的场景（`specs/redesign/room-page.md`）。 */
  readonly layout: RoomPageLayout;
  readonly onExitRoom: () => void;
  readonly onLayoutChange: (layout: RoomPageLayout) => void;
  readonly onSelectedAgentChange: (agentId: string | null) => void;
  readonly onSelectedDirectSessionChange: (catalogId: string | null) => void;
  readonly onSelectedMessageChange: (messageId: string | null) => void;
  readonly onViewChange: (view: RoomWorkspaceView) => void;
  readonly onOpenRoomPanel: (view: RoomPanelView) => void;
  readonly onOpenDoor: (subRoomId: string) => void;
  readonly principal: WebSession | null;
  readonly roomId: string;
  readonly selectedAgentId: string | null;
  readonly selectedDirectSessionId: string | null;
  readonly selectedMessageId: string | null;
  /** `agents` 是对话版面里展开成整页的 Agent 表格；场景里没有那一页，按画面算。 */
  readonly view: RoomPageView;
};

export function LobbyPage(props: LobbyPageProps) {
  const { lobby, messages, messagePublisher, localRuntime } = useAppServices();
  const store = useMemo(
    () => new LobbyExperienceStore(lobby, messages, props.roomId, props.principal),
    [lobby, messages, props.roomId, props.principal?.matrixUserId, props.principal?.displayName],
  );
  const state = useSyncExternalStore(store.subscribe, store.getSnapshot, store.getSnapshot);
  return (
    <ReceptionEvidenceProvider
      gateway={localRuntime}
      principalId={props.principal?.principalId ?? null}
    >
      <ConversationWorkspaceProvider
        publisher={messagePublisher}
        scope={props.principal?.matrixUserId ?? null}
      >
        {state.kind !== 'ready' ? (
          state.kind === 'failed' && state.code === 'lobby.matrix_unavailable' ? (
            // 消息没连上时照会话的状态说、按钮交给会话；连上以后房间自己打开。
            <MatrixConnectionCard
              fallback={<LobbyStateBoundary onRetry={store.retry} state={state} />}
            />
          ) : state.kind === 'failed' && state.code === 'lobby.room_not_joined' ? (
            // 服务器记着你在、Matrix 上还只是受邀（本机 Agent 替你建的房间）：替你加入，不让你回列表找“加入”。
            <PrivateRoomJoinBoundary
              catalogId={props.catalogId}
              onRetry={store.retry}
              principalId={props.principal?.principalId ?? null}
              roomId={props.roomId}
              state={state}
            />
          ) : (
            <LobbyStateBoundary onRetry={store.retry} state={state} />
          )
        ) : (
          <RoomMessagesProvider store={store.messages}>
            <NetworkAgentRelayProvider relayed={state.room.encrypted === true}>
              <AgentOwnersScope
                catalogId={props.catalogId}
                selfPrincipalId={props.principal?.principalId ?? null}
              >
                <ReadyLobby
                  {...props}
                  key={state.room.roomId}
                  room={state.room}
                  scene={state.projection}
                />
              </AgentOwnersScope>
            </NetworkAgentRelayProvider>
          </RoomMessagesProvider>
        )}
      </ConversationWorkspaceProvider>
    </ReceptionEvidenceProvider>
  );
}

function ReadyLobby({
  catalogId,
  door,
  layout,
  onExitRoom,
  onLayoutChange,
  onSelectedAgentChange,
  onSelectedDirectSessionChange,
  onSelectedMessageChange,
  onViewChange,
  onOpenRoomPanel,
  onOpenDoor,
  principal,
  room,
  scene,
  selectedAgentId,
  selectedDirectSessionId,
  selectedMessageId,
  view,
}: LobbyPageProps & { readonly room: LobbyRoom; readonly scene: LobbySceneProjection }) {
  const { t } = useTranslation();
  const {
    config,
    localRuntime,
    messages: messageGateway,
    privateRoomInvites,
    privateRoomListings,
    privateRooms,
    projectRooms,
    roomExcerpts,
    roomLiveViews,
    roomNotices,
    roomPins,
    subRooms,
  } = useAppServices();
  const reception = useReceptionEvidence();
  const { url: downloadUrl } = usePublishedDownload(config);
  const directSessions = useDirectSessionController(principal !== null);
  const [drawer, setDrawer] = useState<'members' | null>(null);
  // 手机上查找收在顶栏的“更多”里：点了就请眼前那段对话打开查找（`turn` 加一），
  // 那段对话也告诉这里此刻有没有可查的。
  const [searchable, setSearchable] = useState(false);
  const [searchTurn, setSearchTurn] = useState(0);
  // 须知下面置顶那一行点了一句：请这间房的对话跳过去（`turn` 加一，同一句点第二次也再跳）。
  const [pinJump, setPinJump] = useState<{
    readonly roomId: string;
    readonly eventId: string;
    readonly turn: number;
  } | null>(null);
  const searchRequests = useMemo(
    () => ({
      turn: searchTurn,
      onAvailable: setSearchable,
      ...(pinJump === null ? {} : { jump: pinJump }),
    }),
    [searchTurn, pinJump],
  );
  // 重新登录回来时，没提交完的自动发言授权要接着填：直接打开房间设置的那一节。
  const [roomSettings, setRoomSettings] = useState<{
    readonly section?: RoomSettingsSection;
  } | null>(() =>
    principal !== null && hasAutomationDraftFor(principal.principalId, catalogId)
      ? { section: 'automation' }
      : null,
  );
  const [inviteOpen, setInviteOpen] = useState(false);
  const [peopleInviteOpen, setPeopleInviteOpen] = useState(false);
  const [shareOpen, setShareOpen] = useState(false);
  const [invitation, setInvitation] = useState<ConnectedInvitation | null>(null);
  const [panelSize, setPanelSize] = useState<PanelSize>(readPanelSize);
  const owner =
    principal === null
      ? null
      : { principalId: principal.principalId, displayName: principal.displayName };
  const membersButton = useRef<HTMLButtonElement>(null);
  const spatial = useRef<LobbySpatialViewHandle>(null);
  // 归档了的私人房间照样打得开、看得到之前的消息，只是谁都不能再说话、也不能再接入 Agent。
  // 子房间不在房间列表里：列表里找不到这一间时单独问一次，接在列表后面一起用。公共大厅也不在列表里，
  // 但它不会是子房间，不去问（问了是 404）。
  const privateRoomList = usePrivateRoomListWith(
    privateRooms,
    principal?.principalId,
    catalogId,
    principal !== null,
    room.publicLobby !== true,
  );
  // 有房间号的房间（私人房间、公开房间）和公共大厅分开：加密的都是；不加密的公开房间在你的房间里，
  // 公共大厅不在（`specs/public-rooms/design.md`）。
  const catalogRoom =
    room.encrypted === true ||
    (privateRoomList.data?.ok === true &&
      privateRoomList.data.value.some((entry) => entry.catalogId === catalogId));
  const origin = usePrivateRoomOrigin(privateRooms, catalogId, principal !== null && catalogRoom);
  const listing = usePrivateRoomListing(
    privateRoomListings,
    catalogId,
    principal !== null && catalogRoom,
  );
  const archived =
    privateRoomList.data?.ok === true &&
    privateRoomList.data.value.some(
      (entry) => entry.catalogId === catalogId && entry.status === 'archived',
    );
  // 绑着 GitHub 仓库的公开房间，房间名旁边一个勾（specs/redesign/project-rooms.md）。房间里的人都看得到：
  // 公开着的房间绑着哪个仓库本来就在 `/gh/` 上公开着，只在公开着的房间里问。
  const repositoryQuery = useRoomRepositoryMark(
    projectRooms,
    catalogId,
    principal !== null && listing.data?.ok === true && listing.data.value.listed,
  );
  const repository =
    repositoryQuery.data?.ok === true && repositoryQuery.data.value !== null
      ? {
          repository: repositoryQuery.data.value.repository,
          url: repositoryQuery.data.value.repositoryUrl,
        }
      : null;
  // 房间“分享”里的“README 徽章”只给房主：那一行在只有房主问得到的绑定那一份里，别人问回 403。勾说绑着仓库
  // 时才去问，没绑的房间不多问一次。
  const repositoryBadge =
    projectRooms !== undefined &&
    repository !== null &&
    principal !== null &&
    privateRoomList.data?.ok === true &&
    privateRoomList.data.value.some(
      (entry) => entry.catalogId === catalogId && entry.ownerPrincipalId === principal.principalId,
    )
      ? projectRooms
      : null;
  // 分享页放着实况时，房间名旁边那一小句说不登录也看得到（specs/redesign/project-rooms.md）。房间里的人都
  // 读得到；只在公开着的房间里问，公共大厅、私聊、私密的房间不问。
  const liveViewQuery = useRoomLiveView(
    roomLiveViews,
    catalogId,
    principal !== null && listing.data?.ok === true && listing.data.value.listed,
  );
  const liveView =
    listing.data?.ok === true &&
    listing.data.value.listed &&
    liveViewQuery.data?.ok === true &&
    liveViewQuery.data.value.live;
  // 私人房间里你没有“自动发言”时，带进来的 Agent 进得来、说不了话：“接入 Agent”里先说。
  const agentsCanSpeak = agentsSpeakIn(
    privateRoomList.data?.ok === true ? privateRoomList.data.value : [],
    catalogId,
    principal?.principalId ?? null,
  );
  // 子房间：邀请链接、往外请人、公开和分享都是整间房才有的（子房间里请人进来在子房间设置里）。
  const familyQuery = useSubRooms(subRooms, catalogId, {
    enabled: principal !== null && catalogRoom,
    managingIntervalMs: SUB_ROOM_WAITING_POLL_MS,
  });
  const family = familyQuery.data?.ok === true ? familyQuery.data.value : null;
  const subRoom = family !== null && isSubRoomOf(family, catalogId);
  // 眼前这间子房间在名单里的那一行（归档了的不在）：子房间设置照它。
  const subRoomEntry = subRoom
    ? family.subRooms.find((entry) => entry.catalogId === catalogId)
    : undefined;
  // 归档了的那一份：地址里的那一间不在没归档的名单里时找它，归档了的子房间页面问能不能恢复。
  const doorActive =
    door === null ? undefined : family?.subRooms.find((entry) => entry.catalogId === door);
  const archivedFamilyQuery = useSubRooms(subRooms, catalogId, {
    archived: true,
    enabled:
      principal !== null &&
      catalogRoom &&
      family !== null &&
      ((door !== null && doorActive === undefined) || (subRoom && archived)),
  });
  const archivedFamily =
    archivedFamilyQuery.data?.ok === true ? archivedFamilyQuery.data.value : null;
  const doorRoom =
    doorActive ??
    (door === null
      ? undefined
      : archivedFamily?.subRooms.find((entry) => entry.catalogId === door));
  // 左栏：“浏览子房间”和你在的子房间（眼前是子房间时它也算）；顶栏和“主房间”回到父房间。
  // 一间都没有时，建得了的人也看得到“浏览子房间”和“新建子房间”。
  const hasSubRoomPlaces = family !== null && (family.subRooms.length > 0 || family.canCreate);
  const [creatingSubRoom, setCreatingSubRoom] = useState(false);
  // 从左栏那一行的“⋯”打开的、不是眼前这一间的子房间设置。
  const [rowSettings, setRowSettings] = useState<SubRoom | null>(null);
  const createSubRoom =
    family?.canCreate === true
      ? () => {
          setCreatingSubRoom(true);
        }
      : null;
  const subRoomParent =
    family !== null && subRoom
      ? {
          catalogId: family.parent.catalogId,
          matrixRoomId:
            (privateRoomList.data?.ok === true
              ? privateRoomList.data.value.find(
                  (entry) => entry.catalogId === family.parent.catalogId,
                )?.matrixRoomId
              : undefined) ?? null,
          name: family.parent.name,
        }
      : null;
  // 每一行的“⋯”：父房间的房主在每一间里，退不出去；退出眼前这一间以后回到父房间。
  const subRoomMenu: SubRoomRowMenuContext | null =
    principal === null || family === null
      ? null
      : {
          principalId: principal.principalId,
          ownerPrincipalId:
            (privateRoomList.data?.ok === true
              ? privateRoomList.data.value.find(
                  (entry) => entry.catalogId === family.parent.catalogId,
                )?.ownerPrincipalId
              : undefined) ?? null,
          parentMatrixRoomId: subRoomParent?.matrixRoomId ?? undefined,
          onOpenSettings: (entry) => {
            if (entry.catalogId === catalogId) setRoomSettings({ section: 'sub-room' });
            else setRowSettings(entry);
          },
        };
  const invitable = subRoom
    ? null
    : invitableRoom(
        privateRoomList.data?.ok === true ? privateRoomList.data.value : [],
        catalogId,
        privateRoomInvites === undefined ? null : (principal?.principalId ?? null),
      );
  // 公开房间（不是只能邀请的、没被平台拿掉）给“分享”：链接打开了别人进得来才给。
  const shareRoom: ShareableRoom | null =
    !archived && !subRoom && listing.data?.ok === true && shareable(listing.data.value)
      ? {
          description:
            (privateRoomList.data?.ok === true
              ? privateRoomList.data.value.find((entry) => entry.catalogId === catalogId)
                  ?.description
              : undefined) ?? '',
          name: room.name,
          shareCode: listing.data.value.shareCode,
          shareUrl: listing.data.value.shareUrl,
        }
      : null;
  // 齿轮上的点：公开房间在等批准的，加上眼前这间子房间在等批准的（你管得了它时名单上才有）。
  const pendingJoinRequests =
    usePendingJoinRequestCount(
      privateRoomList.data?.ok === true ? privateRoomList.data.value : [],
      catalogId,
      principal?.principalId ?? null,
    ) + (subRoomEntry?.pendingRequestCount ?? 0);
  // 能不能发消息看 Matrix 的发言门槛：归档、被禁言、只能查看时输入框都关着，免得发了才说失败。
  const writesAllowed = !archived && room.canSpeak !== false;
  // 房主和管理员能改须知、置顶；归档了谁都不能。
  const managesThisRoom =
    !archived &&
    managesRoom(
      privateRoomList.data?.ok === true ? privateRoomList.data.value : [],
      catalogId,
      principal?.principalId ?? null,
    );
  const noticeEditable = roomNotices !== undefined && managesThisRoom;
  const pinsControl = useRoomPinsActions({
    gateway: roomPins,
    catalogId,
    roomId: room.roomId,
    pins: room.pins,
    manages: managesThisRoom,
    selfMatrixUserId: principal?.matrixUserId ?? null,
  });
  const projection = useMemo(
    () => ({
      ...scene,
      selectedAgentId: scene.nodes.some((agent) => agent.agentId === selectedAgentId)
        ? selectedAgentId
        : null,
    }),
    [scene, selectedAgentId],
  );
  const selectedAgent = room.agents.find((agent) => agent.agentId === selectedAgentId) ?? null;
  const attendance = attendanceCounts(room.agents, room.observedAtUnixMs);
  // 对话版面里对话一直在：地址里不带 `view` 也是对话。场景里不带 `view` 是画面本身，私聊开着时才算对话。
  const conversationLayout = layout === 'conversation';
  // 右栏的表格和顶栏那一行小字用同一份数。
  const roster = useRoomRoster({
    agents: room.agents,
    catalogId,
    observedAtUnixMs: room.observedAtUnixMs,
    principalId: principal?.principalId ?? null,
    room:
      privateRoomList.data?.ok === true
        ? privateRoomList.data.value.find((entry) => entry.catalogId === catalogId)
        : undefined,
    roomId: room.roomId,
    spoken: projection.humans ?? noHumans,
  });
  const sideColumn = useMediaQuery(roomDeskSideQuery);
  // Agent 表格展开成的整页：只在对话版面的主房间里有。开着私聊时地址里写了也先看私聊。
  const agentsPage = conversationLayout && view === 'agents' && selectedDirectSessionId === null;
  // “浏览子房间”：只在对话版面的父房间上有（子房间左栏里那一行去父房间的这一页）。
  // 还没进的一间子房间的卡片：也只在对话版面的父房间上有；名单上找不到这一间（看不到、号不对）就当没写。
  const doorPage =
    conversationLayout &&
    doorRoom !== undefined &&
    selectedDirectSessionId === null &&
    family !== null &&
    !subRoom;
  const subRoomsPage =
    conversationLayout &&
    view === 'sub-rooms' &&
    selectedDirectSessionId === null &&
    family !== null &&
    !subRoom &&
    !doorPage;
  // 展开成整页的东西开着：对话和右栏都不画。
  const fullPage = agentsPage || subRoomsPage || doorPage;
  const unseenSubRooms = useUnseenSubRooms(family, subRoomsPage);
  const localAgentIds = useLocalAgentIds(agentsPage);
  const panel: RoomWorkspaceView = view === 'agents' || view === 'sub-rooms' ? 'space' : view;
  const activeView =
    panel === 'space' && (conversationLayout || selectedDirectSessionId !== null)
      ? 'conversation'
      : panel;
  const { state: roomMessages, store: messageStore } = useRoomMessages(room.roomId);
  // 对话里“分享这一段”（specs/redesign/project-rooms.md“分享一段对话”）：只在放着实况的主房间里有，
  // 也只认开了实况以后的话。
  const timeline = useMemo(
    () => conversationMessages(roomMessages.kind === 'ready' ? roomMessages.room.messages : []),
    [roomMessages],
  );
  const liveSinceUnixMs =
    liveView && !archived && !subRoom && liveViewQuery.data?.ok === true
      ? liveViewQuery.data.value.enabledAtUnixMs
      : null;
  const excerptSharing = useExcerptSharing({
    gateway: principal === null ? undefined : roomExcerpts,
    catalogId,
    roomId: room.roomId,
    liveSinceUnixMs,
    timeline,
  });
  const activityStore = useMemo(
    () => new RoomActivityStore(messageStore, principal?.matrixUserId ?? null),
    [messageStore, principal?.matrixUserId],
  );
  const activity = useSyncExternalStore(
    activityStore.subscribe,
    activityStore.getSnapshot,
    activityStore.getSnapshot,
  );
  const speech = useMemo(
    () => projectRoomSpeech(projection, activity.recent),
    [projection, activity.recent],
  );
  // 展开的那一页上看不到对话：这时来的消息照样算没读，标在回对话的地方。
  const publicConversationVisible =
    activeView === 'conversation' && selectedDirectSessionId === null && !fullPage;
  const [focusedConversationMessageId, setFocusedConversationMessageId] = useState<string | null>(
    null,
  );
  useEffect(() => {
    activityStore.setVisible(publicConversationVisible);
  }, [activityStore, publicConversationVisible]);
  // 窗口不在前台时来的话人没看到：对话就在眼前也记一个数，只写在窗口标题上。
  useEffect(() => {
    const sync = (): void => {
      activityStore.setForeground(windowInForeground());
    };
    sync();
    return onWindowForegroundChange(sync);
  }, [activityStore]);
  useUnreadTitle(activity.unread + activity.away);
  const openRoomPanel = (nextView: RoomPanelView): void => {
    setFocusedConversationMessageId(null);
    onOpenRoomPanel(nextView);
  };
  const openSpeech = (messageId: string): void => {
    onOpenRoomPanel('conversation');
    setFocusedConversationMessageId(messageId);
  };
  const panelView = activeView === 'resources' ? 'resources' : 'conversation';
  useEffect(() => {
    if (selectedAgentId !== null && selectedAgent === null) onSelectedAgentChange(null);
  }, [onSelectedAgentChange, selectedAgent, selectedAgentId]);

  const closeDrawer = (): void => {
    setDrawer(null);
  };
  const roomKind = {
    archived,
    listed: listing.data?.ok === true && listing.data.value.listed,
    live: catalogRoom && liveView,
    privateRoom: catalogRoom,
    serverReadable: origin.data?.ok === true && origin.data.value.ownerIsNetworkAgent,
    unencrypted: catalogRoom && room.encrypted !== true,
  };
  // 换版面以后顶栏整个换了一份：焦点接着落在新顶栏的切换上，不掉回页面开头。
  const layoutChangedHere = useRef(false);
  const changeLayout = (next: RoomPageLayout): void => {
    layoutChangedHere.current = true;
    closeDrawer();
    onLayoutChange(next);
  };
  useEffect(() => {
    if (!layoutChangedHere.current) return;
    layoutChangedHere.current = false;
    // 手机上对话版面的切换收在“更多”里，焦点就落在“更多”上。
    const controls = document.querySelectorAll<HTMLElement>(
      '.room-layout-switch [aria-checked="true"], .room-layout-toggle, .room-desk__more',
    );
    [...controls].find((control) => control.offsetParent !== null)?.focus();
  }, [layout]);
  const selectAgent = (id: string | null): void => {
    closeDrawer();
    onSelectedAgentChange(id);
  };
  // 对话版面里关掉 Agent 详情：焦点回到右栏里刚才那一行（那一行不在这一页上就回到标签）；
  // 右栏收起来了就回到顶栏打开它的按钮。详情开着时表格碰不到，所以等它关了、页面重画完再放焦点。
  const returnFocusFrom = useRef<string | null>(null);
  useEffect(() => {
    const agentId = returnFocusFrom.current;
    if (agentId === null || selectedAgentId !== null) return;
    returnFocusFrom.current = null;
    const origins = [
      ...document.querySelectorAll<HTMLElement>(
        '.room-desk__side .agent-table__name, .room-desk__page .agent-table__name',
      ),
    ].filter((row) => row.dataset.agentId === agentId);
    origins.push(
      ...document.querySelectorAll<HTMLElement>(
        '.room-desk__side [role="tab"][aria-selected="true"], .room-desk__agents-button, .room-desk__page input[type="search"]',
      ),
    );
    origins.find((origin) => origin.offsetParent !== null)?.focus();
  }, [selectedAgentId]);
  // 展开成整页、从那一页回来，原来拿着焦点的东西都不在了：展开时焦点落到那一页的标题上，
  // 点顶栏的房间名回来时落回打开它的地方：Agent 表格是“展开成整页”（右栏收着时是顶栏打开它的按钮），
  // “浏览子房间”是左栏那一行（左栏收着时是房间名旁边的下拉）。
  const pageFocus = useRef<'opened' | 'closed' | null>(null);
  const closedPage = useRef<'agents' | 'sub-rooms'>('agents');
  useEffect(() => {
    const moved = pageFocus.current;
    pageFocus.current = null;
    if (moved === 'opened' && fullPage)
      document.querySelector<HTMLElement>('.room-desk__page h2')?.focus();
    if (moved === 'closed' && !fullPage) {
      const origins = document.querySelectorAll<HTMLElement>(
        closedPage.current === 'agents'
          ? '.room-desk__side .room-roster__expand, .room-desk__agents-button'
          : '.room-desk__rail .sub-room-places__browse, .room-desk__place-trigger',
      );
      [...origins].find((origin) => origin.offsetParent !== null)?.focus();
    }
  }, [agentsPage, doorPage, fullPage, subRoomsPage]);
  // 私聊的列表。对话版面里它在左栏，窄了在房间名旁边的下拉里（点了一项先把下拉关上：`before`）；
  // 场景里它在 Agent 抽屉里。
  const directNavigation = (before?: () => void) =>
    principal === null ? null : (
      <DirectSessionNavigation
        activeCatalogId={selectedDirectSessionId}
        controller={directSessions}
        onActivate={(id) => {
          before?.();
          closeDrawer();
          onSelectedDirectSessionChange(id);
        }}
      />
    );
  const directName =
    selectedDirectSessionId === null
      ? null
      : (directSessions.sessions.find((session) => session.catalogId === selectedDirectSessionId)
          ?.target.displayName ?? t('roomWorkspace.direct'));
  // 归档了的房间不再管 Agent 离线多久归档，也就不问管不管得了（问了回 404）。私人房间等列表到了
  // 再看，免得列表还没到时先问一次；公开大厅不在私人房间列表里。
  const roomStatusKnown = room.publicLobby === true || privateRoomList.data !== undefined;
  const rosterPolicy =
    archived || !roomStatusKnown ? null : (
      <AgentRosterPolicyPanel
        key={`${catalogId}:${String(room.archiveAfterDays ?? 7)}`}
        catalogId={catalogId}
        days={room.archiveAfterDays ?? 7}
      />
    );
  // 对话版面是表格（`RoomRoster`），右栏和窄的时候的抽屉里都只列 Agent 和人：私聊在左栏，
  // 窄了在房间名旁边的下拉里。场景里照旧是原来那份紧凑的名册，抽屉里连私聊一起列。
  const rosterPanel = () =>
    conversationLayout ? (
      <div className="workspace-members">
        <RoomRoster
          agents={room.agents}
          footer={rosterPolicy}
          observedAtUnixMs={room.observedAtUnixMs}
          onInviteAgent={
            archived
              ? null
              : () => {
                  closeDrawer();
                  setInviteOpen(true);
                }
          }
          onExpand={() => {
            pageFocus.current = 'opened';
            closeDrawer();
            openRoomPanel('agents');
          }}
          onSelectAgent={selectAgent}
          people={roster.people}
          selectedAgentId={selectedAgentId}
          wakeableAgentIds={roster.wakeableAgentIds}
        />
      </div>
    ) : (
      <div className="workspace-members">
        <ListModeRoster
          agents={room.agents}
          observedAtUnixMs={room.observedAtUnixMs}
          onSelectAgent={selectAgent}
          selectedAgentId={selectedAgentId}
          variant="compact"
        />
        <p className="workspace-members__note">{t('roomWorkspace.memberNote')}</p>
        {directNavigation()}
        {rosterPolicy}
      </div>
    );
  const layoutSwitch = <RoomLayoutSwitch onChange={changeLayout} value={layout} />;
  const dialogs = (
    <>
      {creatingSubRoom && family !== null ? (
        <NewSubRoomDialog
          list={family}
          onClose={() => {
            setCreatingSubRoom(false);
          }}
          roomAgents={room.agents}
        />
      ) : null}
      {shareOpen && shareRoom !== null ? (
        <ShareRoomDialog
          onClose={() => {
            setShareOpen(false);
          }}
          room={shareRoom}
        >
          {repositoryBadge === null ? null : (
            <RepositoryBadgeShare catalogId={catalogId} gateway={repositoryBadge} />
          )}
          {roomExcerpts === undefined || principal === null ? null : (
            <SharedExcerpts
              catalogId={catalogId}
              gateway={roomExcerpts}
              live={liveView}
              manages={managesThisRoom}
            />
          )}
        </ShareRoomDialog>
      ) : null}
      {excerptSharing.dialog === null || excerptSharing.selected === null ? null : (
        <ShareExcerptDialog
          messages={excerptSharing.selected}
          onClose={excerptSharing.closeDialog}
          onRetry={excerptSharing.retry}
          roomName={room.name}
          state={excerptSharing.dialog}
        />
      )}
      {roomSettings === null || principal === null ? null : (
        <RoomSettingsDialog
          catalogId={catalogId}
          initialSection={roomSettings.section}
          onClose={() => {
            setRoomSettings(null);
          }}
          memberNames={room.memberNames}
          notice={room.notice}
          onExitRoom={onExitRoom}
          principal={principal}
          roomName={room.name}
          subRoom={
            family !== null && subRoomEntry !== undefined
              ? { list: family, room: subRoomEntry }
              : null
          }
        />
      )}
      {rowSettings === null || principal === null || family === null ? null : (
        <RoomSettingsDialog
          catalogId={rowSettings.catalogId}
          onClose={() => {
            setRowSettings(null);
          }}
          onExitRoom={() => {
            setRowSettings(null);
          }}
          principal={principal}
          roomName={rowSettings.name}
          subRoom={{
            list: family,
            room:
              family.subRooms.find((entry) => entry.catalogId === rowSettings.catalogId) ??
              rowSettings,
          }}
          subRoomOnly
        />
      )}
      {peopleInviteOpen &&
      invitable !== null &&
      principal !== null &&
      privateRoomInvites !== undefined ? (
        <PrivateRoomInviteDialog
          inviterName={principal.displayName}
          invites={privateRoomInvites}
          onClose={() => {
            setPeopleInviteOpen(false);
          }}
          principalId={principal.principalId}
          room={invitable}
          rooms={privateRooms}
        />
      ) : null}
      {inviteOpen ? (
        <AgentInviteDialog
          agentsCanSpeak={agentsCanSpeak}
          onConnected={setInvitation}
          onStartConversation={() => {
            setInviteOpen(false);
            onOpenRoomPanel('conversation');
          }}
          downloadUrl={downloadUrl}
          onClose={() => {
            setInviteOpen(false);
          }}
          owner={owner}
          presentAgents={room.agents}
          room={{ catalogId, roomId: room.roomId, roomName: room.name }}
        />
      ) : null}
    </>
  );
  const conversationNotices = (
    <>
      {writesAllowed || selectedDirectSessionId !== null ? null : archived && subRoom ? (
        <ArchivedSubRoomNotice
          list={family}
          room={archivedFamily?.subRooms.find((entry) => entry.catalogId === catalogId)}
        />
      ) : (
        <p className="room-panel__read-only" role="status">
          {archived ? <Archive aria-hidden="true" /> : <MessageSquareOff aria-hidden="true" />}
          {t(archived ? 'roomWorkspace.archivedNotice' : 'roomWorkspace.readOnlyNotice')}
        </p>
      )}
      {publicConversationVisible && invitation !== null && principal !== null ? (
        <div className="room-first-reply">
          <strong>{invitation.displayName}</strong>
          <InviteReplyProgress
            gateway={messageGateway}
            agentId={invitation.agentId}
            roomId={invitation.roomId}
            startedAt={invitation.startedAt}
            principalId={principal.principalId}
          />
          <button
            type="button"
            className="ar-icon-button"
            aria-label={t('agentInvite.close')}
            onClick={() => {
              setInvitation(null);
            }}
          >
            <X aria-hidden="true" />
          </button>
        </div>
      ) : null}
      {publicConversationVisible ? (
        <RoomBriefs
          onEdit={
            noticeEditable && principal !== null
              ? () => {
                  setRoomSettings({ section: 'notice' });
                }
              : undefined
          }
          parent={subRoomParent}
          room={room}
          selfMatrixUserId={principal?.matrixUserId ?? null}
        />
      ) : null}
      {publicConversationVisible ? (
        <RoomPinsBar
          actions={pinsControl.actions}
          failure={pinsControl.failure}
          key={`pins:${room.roomId}`}
          onDismissFailure={pinsControl.dismissFailure}
          onJump={(eventId) => {
            setPinJump((current) => ({
              roomId: room.roomId,
              eventId,
              turn: (current?.turn ?? 0) + 1,
            }));
          }}
          pins={pinsControl.pins}
          roomId={room.roomId}
          selfMatrixUserId={principal?.matrixUserId ?? null}
        />
      ) : null}
      {publicConversationVisible && excerptSharing.selected !== null ? (
        <ExcerptSelectionBar
          count={excerptSharing.selected.length}
          onCancel={excerptSharing.cancel}
          onShare={excerptSharing.share}
          sharing={excerptSharing.dialog?.kind === 'pending'}
          tooLong={excerptSharing.tooLong}
        />
      ) : null}
    </>
  );
  // 对话版面里没有“对话 / 资料”两个标签：换过去、换回来的按钮和须知都放进对话那一栏里。
  const deskViews = conversationLayout ? { onViewChange } : {};
  // 对话版面和场景版面都放在这里面：手机上“更多”里的查找、置顶那一行点的一句都由它交给对话。
  const conversationContent = (
    <ConversationSearchRequestContext.Provider value={searchRequests}>
      <RoomPinsActionsContext.Provider value={pinsControl.actions}>
        <ExcerptSelectionContext.Provider value={excerptSharing.actions}>
          <div
            {...(conversationLayout
              ? { className: 'room-desk__content' }
              : {
                  className: 'room-panel__content',
                  id: 'workspace-current-view',
                  role: 'tabpanel',
                  'aria-labelledby': `workspace-tab-${panelView}`,
                })}
          >
            <div className="workspace-room-content" hidden={selectedDirectSessionId !== null}>
              <MessageLayer
                {...deskViews}
                {...(conversationLayout
                  ? { conversationUnread: activity.unread, notices: conversationNotices }
                  : {})}
                active={publicConversationVisible}
                focusedConversationMessageId={focusedConversationMessageId}
                participants={room.agents}
                {...(room.memberNames === undefined ? {} : { memberNames: room.memberNames })}
                catalogId={catalogId}
                onSelectedMessageChange={onSelectedMessageChange}
                roomId={room.roomId}
                roomName={room.name}
                privateRoom={room.encrypted === true}
                selectedMessageId={selectedDirectSessionId === null ? selectedMessageId : null}
                view={panelView}
                writesAllowed={writesAllowed}
              />
            </div>
            <DirectConversationDock
              {...deskViews}
              activeCatalogId={selectedDirectSessionId}
              controller={directSessions}
              onSelectedMessageChange={onSelectedMessageChange}
              selectedMessageId={selectedMessageId}
              view={panelView}
            />
          </div>
        </ExcerptSelectionContext.Provider>
      </RoomPinsActionsContext.Provider>
    </ConversationSearchRequestContext.Provider>
  );
  const agentDrawer =
    drawer === null ? null : (
      <WorkspaceDrawer
        label={t('roomWorkspace.members')}
        // 对话版面在手机上是从下面出来的一整页；场景版面照旧从右边出来。
        sheet={conversationLayout}
        variant={drawer}
        onClose={closeDrawer}
      >
        {rosterPanel()}
      </WorkspaceDrawer>
    );
  const inspector = (
    <AnimatePresence>
      {selectedAgent === null ? null : (
        <AgentInspector
          hasBackgroundReception={
            roster.wakeableAgentIds.has(selectedAgent.agentId) ||
            reception.some(
              (view) =>
                view.state.agentId === selectedAgent.agentId &&
                view.state.binding.policy.roomId === room.roomId,
            )
          }
          wakeable={roster.wakeableAgentIds.has(selectedAgent.agentId)}
          receptionControls={
            principal !== null && localRuntime.isAvailable() ? (
              <ReceptionPanel agentId={selectedAgent.agentId} roomId={room.roomId} />
            ) : principal !== null ? (
              <ReceptionOwnershipPanel agentId={selectedAgent.agentId} roomId={room.roomId} />
            ) : undefined
          }
          actionFailure={inspectorFailure(directSessions)}
          agent={selectedAgent}
          blocked={directSessions.isBlocked(selectedAgent)}
          // 右栏在的时候详情盖在表格上，关掉就是回到表格；右栏收起来时它是从右边出来的一层。
          // 展开的那一页上没有右栏，详情也是从右边出来的一层。
          closeAs={conversationLayout && sideColumn && !fullPage ? 'back' : 'close'}
          observedAtUnixMs={room.observedAtUnixMs}
          key="agent-inspector"
          pendingAction={
            directSessions.opening ? 'message' : directSessions.blocking ? 'block' : null
          }
          // 屏蔽完详情留着：上面说已屏蔽，按钮换成解除屏蔽，点错了马上能改回来。
          onBlock={() => {
            void directSessions.setBlocked(selectedAgent, true);
          }}
          onUnblock={() => {
            void directSessions.setBlocked(selectedAgent, false);
          }}
          onClose={() => {
            if (conversationLayout) returnFocusFrom.current = selectedAgent.agentId;
            else spatial.current?.focus();
            onSelectedAgentChange(null);
          }}
          onMessage={(agentId) => {
            void directSessions.openAgent(agentId).then((result) => {
              if (result.ok) {
                onSelectedAgentChange(null);
                onSelectedDirectSessionChange(result.value.catalogId);
              }
            });
          }}
        />
      )}
    </AnimatePresence>
  );

  if (conversationLayout)
    return (
      <>
        <RoomDesk
          agentCount={roster.agentCount}
          agentFaces={roster.faces}
          directName={directName}
          inspector={inspector}
          inspecting={selectedAgent !== null}
          inviteAgent={
            archived
              ? null
              : {
                  open: inviteOpen,
                  onOpen: () => {
                    setInviteOpen(true);
                  },
                }
          }
          invitePeople={
            invitable === null
              ? null
              : {
                  open: peopleInviteOpen,
                  onOpen: () => {
                    setPeopleInviteOpen(true);
                  },
                }
          }
          kind={roomKind}
          layoutSwitch={layoutSwitch}
          liveSummary={t('lobby.liveSummary', { count: attendance.present, room: room.name })}
          mainRoomSelected={selectedDirectSessionId === null && !fullPage}
          mainRoomUnread={activity.unread}
          onOpenAgents={() => {
            setDrawer('members');
          }}
          onOpenSettings={
            principal === null
              ? null
              : () => {
                  setRoomSettings({});
                }
          }
          onSearch={
            searchable && panelView === 'conversation' && !fullPage
              ? () => {
                  setSearchTurn((turn) => turn + 1);
                }
              : null
          }
          onSelectMainRoom={() => {
            openRoomPanel('conversation');
          }}
          onShowScene={() => {
            changeLayout('scene');
          }}
          page={
            agentsPage
              ? {
                  title: t('roomRoster.tab.agents'),
                  onClose: () => {
                    pageFocus.current = 'closed';
                    closedPage.current = 'agents';
                    openRoomPanel('conversation');
                  },
                  content: (
                    <AgentTablePage
                      agents={room.agents}
                      footer={rosterPolicy}
                      localAgentIds={localAgentIds}
                      observedAtUnixMs={room.observedAtUnixMs}
                      onInviteAgent={
                        archived
                          ? null
                          : () => {
                              setInviteOpen(true);
                            }
                      }
                      onSelectAgent={selectAgent}
                      selectedAgentId={selectedAgentId}
                      wakeableAgentIds={roster.wakeableAgentIds}
                    />
                  ),
                }
              : doorPage
                ? {
                    title: doorRoom.name,
                    aboutAnotherRoom: true,
                    onClose: () => {
                      pageFocus.current = 'closed';
                      closedPage.current = 'sub-rooms';
                      openRoomPanel('conversation');
                    },
                    content: <SubRoomDoor list={family} room={doorRoom} />,
                  }
                : subRoomsPage
                  ? {
                      title: t('subRooms.browse.title'),
                      onClose: () => {
                        pageFocus.current = 'closed';
                        closedPage.current = 'sub-rooms';
                        openRoomPanel('conversation');
                      },
                      content: (
                        <SubRoomBrowser
                          list={family}
                          onCreate={createSubRoom}
                          onOpenSettings={subRoomMenu?.onOpenSettings ?? null}
                          onOpenDoor={(subRoomId) => {
                            pageFocus.current = 'opened';
                            onOpenDoor(subRoomId);
                          }}
                          principalId={principal?.principalId ?? null}
                        />
                      ),
                    }
                  : null
          }
          pendingJoinRequests={pendingJoinRequests}
          peopleCount={roster.people.people.length}
          parent={subRoomParent}
          rail={
            directSessions.sessions.length === 0 && !hasSubRoomPlaces
              ? null
              : (onNavigate) => (
                  <>
                    {family === null ? null : (
                      <SubRoomPlaces
                        browse={
                          !hasSubRoomPlaces
                            ? null
                            : {
                                count: family.subRooms.length,
                                onOpen: subRoom
                                  ? null
                                  : () => {
                                      pageFocus.current = 'opened';
                                      openRoomPanel('sub-rooms');
                                    },
                                parentMatrixRoomId: subRoomParent?.matrixRoomId ?? undefined,
                                selected: subRoomsPage,
                                unseen: unseenSubRooms,
                              }
                        }
                        currentCatalogId={catalogId}
                        currentSelected={selectedDirectSessionId === null && !fullPage}
                        currentUnread={activity.unread}
                        list={family}
                        menu={subRoomMenu}
                        onCreate={createSubRoom}
                        onNavigate={onNavigate}
                        onSelectCurrent={() => {
                          openRoomPanel('conversation');
                        }}
                      />
                    )}
                    {directSessions.sessions.length === 0 ? null : directNavigation(onNavigate)}
                  </>
                )
          }
          repository={repository}
          roomName={room.name}
          share={
            shareRoom === null
              ? null
              : {
                  open: shareOpen,
                  onOpen: () => {
                    setShareOpen(true);
                  },
                }
          }
          side={rosterPanel()}
          view={panelView}
        >
          {conversationContent}
        </RoomDesk>
        {dialogs}
        {agentDrawer}
      </>
    );

  return (
    <main className="lobby-workspace lobby-game" id="main-content" data-view={activeView}>
      <p aria-atomic="true" aria-live="polite" className="sr-only">
        {t('lobby.liveSummary', { count: attendance.present, room: room.name })}
      </p>
      <div className="room-scene">
        <LobbySpatialView
          room={room}
          projection={projection}
          speech={speech}
          onOpenSpeech={openSpeech}
          onSelectHuman={() => {
            openRoomPanel('conversation');
          }}
          selectedAgentId={selectedAgentId}
          onSelectAgent={selectAgent}
          ref={spatial}
        />
      </div>
      <RoomBeacon
        agentCount={attendance.present}
        {...roomKind}
        layoutSwitch={layoutSwitch}
        membersButtonRef={membersButton}
        repository={repository}
        roomName={room.name}
        onOpenSettings={
          principal === null
            ? null
            : () => {
                setRoomSettings({});
              }
        }
        onOpenMembers={() => {
          setDrawer('members');
        }}
        pendingJoinRequests={pendingJoinRequests}
        {...(room.topic === undefined ? {} : { topic: room.topic })}
      />
      <p className="room-scene-hint">{t('roomGame.hint')}</p>
      {attendance.present === 0 && !archived ? (
        <div className="room-empty-presence" role="status">
          <strong>
            {t(attendance.reconnecting > 0 ? 'studio.reconnectingRoom' : 'studio.emptyRoom')}
          </strong>
          <p>{t('studio.emptyHint')}</p>
          <div className="room-empty-presence__actions">
            <button
              data-tone="primary"
              type="button"
              onClick={() => {
                setInviteOpen(true);
              }}
            >
              <Bot aria-hidden="true" />
              {t('studio.inviteAgent')}
            </button>
            <button
              type="button"
              onClick={() => {
                setDrawer('members');
              }}
            >
              {t('studio.findAway')}
            </button>
          </div>
        </div>
      ) : null}
      <div className="room-human-presence" role="group" aria-label={t('roomGame.people')}>
        {(projection.humans ?? []).slice(0, 3).map((human) => (
          <button
            type="button"
            key={human.matrixUserId}
            onClick={() => {
              openRoomPanel('conversation');
            }}
            aria-label={t(human.isSelf ? 'roomGame.selfCharacter' : 'roomGame.humanCharacter', {
              name: human.displayName,
            })}
          >
            <UsersRound aria-hidden="true" />
            <span>{human.isSelf ? t('roomGame.self') : human.displayName}</span>
          </button>
        ))}
      </div>
      <nav className="room-toolbelt" aria-label={t('roomGame.actions')}>
        <button
          type="button"
          aria-pressed={activeView === 'conversation' && selectedDirectSessionId === null}
          onClick={() => {
            openRoomPanel('conversation');
          }}
        >
          <MessageCircle aria-hidden="true" />
          <span className="room-toolbelt__label">{t('roomGame.chat')}</span>
          <span className="room-toolbelt__short" aria-hidden="true">
            {t('roomGame.chatShort')}
          </span>
          {activity.unread === 0 ? null : (
            <span
              className="room-unread"
              aria-label={t('roomGame.unread', { count: activity.unread })}
            >
              {activity.unread > 99 ? '99+' : activity.unread}
            </span>
          )}
        </button>
        <button
          type="button"
          aria-pressed={activeView === 'resources' && selectedDirectSessionId === null}
          onClick={() => {
            openRoomPanel('resources');
          }}
        >
          <Files aria-hidden="true" />
          <span className="room-toolbelt__label">{t('roomGame.resources')}</span>
          <span className="room-toolbelt__short" aria-hidden="true">
            {t('roomGame.resourcesShort')}
          </span>
        </button>
        <button
          type="button"
          onClick={() => {
            setDrawer('members');
          }}
        >
          <UsersRound aria-hidden="true" />
          <span className="room-toolbelt__label">{t('roomGame.characters')}</span>
          <span className="room-toolbelt__short" aria-hidden="true">
            {t('roomGame.charactersShort')}
          </span>
        </button>
        {archived ? null : (
          <button
            type="button"
            aria-haspopup="dialog"
            aria-expanded={inviteOpen}
            onClick={() => {
              setInviteOpen(true);
            }}
          >
            <Bot aria-hidden="true" />
            <span className="room-toolbelt__label">{t('agentInvite.open')}</span>
            <span className="room-toolbelt__short" aria-hidden="true">
              {t('roomGame.inviteShort')}
            </span>
          </button>
        )}
        {invitable === null ? null : (
          <button
            type="button"
            aria-haspopup="dialog"
            aria-expanded={peopleInviteOpen}
            onClick={() => {
              setPeopleInviteOpen(true);
            }}
          >
            <UserPlus aria-hidden="true" />
            <span className="room-toolbelt__label">{t('privateRooms.inviteLink.open')}</span>
            <span className="room-toolbelt__short" aria-hidden="true">
              {t('privateRooms.inviteLink.openShort')}
            </span>
          </button>
        )}
        {shareRoom === null ? null : (
          <button
            type="button"
            aria-haspopup="dialog"
            aria-expanded={shareOpen}
            onClick={() => {
              setShareOpen(true);
            }}
          >
            <Share2 aria-hidden="true" />
            <span className="room-toolbelt__label">{t('roomShare.open')}</span>
            <span className="room-toolbelt__short" aria-hidden="true">
              {t('roomShare.open')}
            </span>
          </button>
        )}
      </nav>
      {dialogs}
      <section
        className="room-panel"
        data-size={panelSize}
        hidden={activeView === 'space'}
        aria-label={t('roomGame.panel')}
      >
        <header className="room-panel__header">
          <WorkspaceViewTabs value={panelView} onChange={onViewChange} allowSpace={false} />
          <div className="room-panel__header-actions">
            <button
              type="button"
              className="ar-icon-button room-panel__widen"
              aria-label={t(panelSize === 'wide' ? 'roomGame.narrowPanel' : 'roomGame.widenPanel')}
              title={t(panelSize === 'wide' ? 'roomGame.narrowPanel' : 'roomGame.widenPanel')}
              onClick={() => {
                const next = panelSize === 'wide' ? 'regular' : 'wide';
                setPanelSize(next);
                writePanelSize(next);
              }}
            >
              {panelSize === 'wide' ? (
                <Minimize2 aria-hidden="true" />
              ) : (
                <Maximize2 aria-hidden="true" />
              )}
            </button>
            <button
              type="button"
              className="ar-icon-button"
              aria-label={t('roomGame.closePanel')}
              onClick={() => {
                onViewChange('space');
                spatial.current?.focus();
              }}
            >
              <X aria-hidden="true" />
            </button>
          </div>
        </header>
        {conversationNotices}
        {conversationContent}
      </section>
      {agentDrawer}
      {inspector}
    </main>
  );
}

/** 详情里只说屏蔽、解除屏蔽、私聊没做成；读私聊列表出的错在私聊那边说。 */
function inspectorFailure(controller: DirectSessionController): AgentInspectorFailure | null {
  if (controller.failure === null || controller.failedAction === null) return null;
  return {
    action: controller.failedAction === 'open' ? 'message' : controller.failedAction,
    code: controller.failure.code,
  };
}

/**
 * 能在这个房间里邀请人时是这个私人房间：房主，或者有“邀请”权限的已加入成员；归档了的不行
 * （`specs/room-people/design.md`）。没有邀请链接服务（测试、夹具）时也没有。
 */
function invitableRoom(
  rooms: readonly PrivateRoom[],
  catalogId: string,
  principalId: string | null,
): PrivateRoom | null {
  if (principalId === null) return null;
  const room = rooms.find((entry) => entry.catalogId === catalogId);
  if (room?.status !== 'active') return null;
  return room.ownerPrincipalId === principalId || allows(memberFor(room, principalId), 'invite')
    ? room
    : null;
}

/** 你在这个私人房间里带进来的 Agent 能不能说话：房主都能，成员要有“自动发言”。不是私人房间的都能。 */
/** 你是这间房的房主，或者有“管理”。公共大厅和你不在里面的房间都不算。 */
function managesRoom(
  rooms: readonly PrivateRoom[],
  catalogId: string,
  principalId: string | null,
): boolean {
  const room = rooms.find((entry) => entry.catalogId === catalogId);
  if (room === undefined || principalId === null) return false;
  return room.ownerPrincipalId === principalId || allows(memberFor(room, principalId), 'manage');
}

function agentsSpeakIn(
  rooms: readonly PrivateRoom[],
  catalogId: string,
  principalId: string | null,
): boolean {
  const room = rooms.find((entry) => entry.catalogId === catalogId);
  if (room === undefined || principalId === null || room.ownerPrincipalId === principalId) {
    return true;
  }
  return allows(memberFor(room, principalId), 'automate');
}
