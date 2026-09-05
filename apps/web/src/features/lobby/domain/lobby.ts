import type { Result } from '@/shared/result';

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
  readonly summary?: string;
  readonly trust: LobbyAgentTrust;
  readonly visibility: LobbyAgentVisibility;
};

export type LobbyRoom = {
  readonly joinedMemberIds?: readonly string[];
  readonly agents: readonly LobbyAgent[];
  readonly name: string;
  readonly observedAtUnixMs: number;
  readonly roomId: string;
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
