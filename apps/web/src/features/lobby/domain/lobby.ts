import type { Result } from '@/shared/result';
import type { AgentLifecycle, MatrixPresenceObservation } from '@agent-room/protocol';

export const lobbyAgentStatuses = [
  'offline',
  'idle',
  'working',
  'waiting_input',
  'blocked',
  'completed',
] as const;

export type LobbyAgentStatus = (typeof lobbyAgentStatuses)[number];
export type LobbyAgentTrust = 'unknown' | 'verified';
export type LobbyAgentVisibility = 'coarse' | 'detailed';

export type LobbyAgent = {
  readonly agentId: string;
  readonly avatarUrl?: string;
  readonly displayName: string;
  readonly instanceIds: readonly string[];
  readonly matrixUserId: string;
  readonly status: LobbyAgentStatus;
  readonly statusExpiresAtUnixMs: number;
  readonly reportedStatus?: LobbyAgentStatus;
  /** 状态事件的时间；名片就是写名片的时间，不是上次连接。 */
  readonly lastActiveAtUnixMs?: number;
  readonly lastPolledAtUnixMs?: number;
  readonly listeningUntilUnixMs?: number | null;
  /**
   * `presence`：状态事件只是名片，在不在线、在不在等消息看 Matrix 的在线状态
   * （`specs/agent-liveness/design.md`）。不写就是旧的租约写法。
   */
  readonly liveness?: 'presence';
  /** 名片 Agent 的 Matrix 在线状态；还没拿到就不写。 */
  readonly presence?: MatrixPresenceObservation;
  readonly lifecycle?: AgentLifecycle;
  readonly trust: LobbyAgentTrust;
  readonly visibility: LobbyAgentVisibility;
};

/** 房间成员在 Matrix 上的名字，按 `member-names.ts` 的规则处理过。 */
export type LobbyMemberName = {
  readonly displayName: string;
  readonly matrixUserId: string;
};

export type LobbyRoom = {
  /** 你此刻能不能在这里发消息：房间归档、你被禁言或只能查看时不能。不给算能。 */
  readonly canSpeak?: boolean;
  /** 私人房间都是端到端加密的；公开大厅不加密。 */
  readonly encrypted?: boolean;
  readonly joinedMemberIds?: readonly string[];
  /**
   * 已加入的成员叫什么，只有私人房间有：Agent 点了没说过话的人时，对话里靠它写出名字。
   * 公开大厅不给成员名字（`specs/room-people/design.md`）。没设名字的人不在里面。
   */
  readonly memberNames?: readonly LobbyMemberName[];
  readonly agents: readonly LobbyAgent[];
  readonly name: string;
  readonly observedAtUnixMs: number;
  readonly roomId: string;
  readonly archiveAfterDays?: number;
  readonly topic?: string;
};

export type LobbyFailureCode =
  'lobby.matrix_unavailable' | 'lobby.room_not_joined' | 'lobby.room_projection_invalid';

export type LobbyFailure = {
  readonly code: LobbyFailureCode;
  readonly retryable: boolean;
};

export type LobbyReadResult = Result<LobbyRoom, LobbyFailure>;

export type LobbyGateway = {
  read(roomId: string): LobbyReadResult;
  subscribe(roomId: string, listener: () => void): () => void;
};
