import { evaluateAgentStatusLease } from '@agent-room/protocol/status-lease';
import { z } from 'zod';

import type { MatrixLobbyRoomSnapshot, MatrixLobbySource } from './matrix-lobby-source';
import {
  lobbyAgentStatuses,
  type LobbyAgent,
  type LobbyAgentStatus,
  type LobbyGateway,
  type LobbyReadResult,
  type LobbyRoom,
} from '@/features/lobby/domain/lobby';
import { err, ok } from '@/shared/result';

const uuidV7Schema = z
  .string()
  .regex(/^[0-9a-f]{8}-[0-9a-f]{4}-7[0-9a-f]{3}-[89ab][0-9a-f]{3}-[0-9a-f]{12}$/u);
const matrixUserIdSchema = z
  .string()
  .min(4)
  .max(255)
  .regex(/^@[^:]+:[^:]+$/u);
const actorAgentSchema = z
  .looseObject({
    agentId: uuidV7Schema,
    avatarUrl: z
      .string()
      .max(2_048)
      .regex(/^https:\/\//u)
      .optional(),
    displayName: z.string().min(1).max(80),
    matrixUserId: matrixUserIdSchema,
  })
  .superRefine(limitProperties(16));
const actorSchema = z
  .looseObject({
    agent: actorAgentSchema,
    instanceId: uuidV7Schema,
    provenance: z.enum(['human', 'human_confirmed_agent', 'autonomous_agent']),
  })
  .superRefine(limitProperties(12));
const statusEventSchema = z
  .looseObject({
    actor: actorSchema,
    correlationId: z.uuid(),
    createdAt: z.iso.datetime({ offset: true }),
    eventType: z.literal('io.github.rainyflash.agentroom.agent.status.v1'),
    id: uuidV7Schema,
    leaseExpiresAt: z.iso.datetime({ offset: true }),
    progress: z.number().min(0).max(1).optional(),
    schemaVersion: z.literal('1.0'),
    signature: z
      .string()
      .min(43)
      .max(128)
      .regex(/^[A-Za-z0-9_-]+$/u),
    startedAt: z.iso.datetime({ offset: true }).optional(),
    status: z.enum(lobbyAgentStatuses),
    taskSummary: z.string().min(1).max(160).optional(),
    visibility: z.enum(['coarse', 'detailed']),
  })
  .superRefine((event, context) => {
    limitProperties(24)(event, context);
    if (
      event.visibility === 'coarse' &&
      (event.taskSummary !== undefined ||
        event.startedAt !== undefined ||
        event.progress !== undefined)
    ) {
      context.addIssue({ code: 'custom', message: '粗粒度状态不得携带任务详情。' });
    }
  });

type ParsedStatusEvent = z.output<typeof statusEventSchema>;

type AgentCandidate = {
  readonly createdAtUnixMs: number;
  readonly event: ParsedStatusEvent;
  readonly expiresAtUnixMs: number;
  readonly status: LobbyAgentStatus;
};

const STATUS_PRIORITY: Readonly<Record<LobbyAgentStatus, number>> = Object.freeze({
  blocked: 5,
  completed: 2,
  idle: 1,
  offline: 0,
  waiting_input: 4,
  working: 3,
});

const STATUS_LEASE_POLICY = Object.freeze({
  allowedClockSkewMs: 15_000,
  maximumLeaseMs: 300_000,
});

export class MatrixLobbyGateway implements LobbyGateway {
  readonly #now: () => number;
  readonly #source: MatrixLobbySource;

  constructor(source: MatrixLobbySource, now: () => number = Date.now) {
    this.#source = source;
    this.#now = now;
  }

  read(roomId: string): LobbyReadResult {
    try {
      const read = this.#source.read(roomId);
      if (read.kind === 'matrix-unavailable') {
        return err({ code: 'lobby.matrix_unavailable', retryable: true });
      }
      if (read.kind === 'room-not-joined') {
        return err({ code: 'lobby.room_not_joined', retryable: true });
      }
      const observedAtUnixMs = this.#now();
      return ok(projectRoom(read.room, observedAtUnixMs));
    } catch {
      return err({ code: 'lobby.room_projection_invalid', retryable: true });
    }
  }

  subscribe(roomId: string, listener: () => void): () => void {
    return this.#source.subscribe(roomId, listener);
  }
}

function projectRoom(room: MatrixLobbyRoomSnapshot, observedAtUnixMs: number): LobbyRoom {
  const joinedMembers = new Set(room.joinedMemberIds);
  const candidatesByAgent = new Map<string, AgentCandidate[]>();
  for (const stateEvent of room.statusEvents) {
    const parsed = statusEventSchema.safeParse(stateEvent.content);
    if (!parsed.success) {
      continue;
    }
    const event = parsed.data;
    if (
      stateEvent.stateKey !== event.actor.instanceId ||
      stateEvent.sender !== event.actor.agent.matrixUserId ||
      !joinedMembers.has(event.actor.agent.matrixUserId)
    ) {
      continue;
    }
    const lease = evaluateAgentStatusLease(
      event,
      observedAtUnixMs,
      observedAtUnixMs,
      STATUS_LEASE_POLICY,
    );
    if (!lease.ok) {
      continue;
    }
    const candidate: AgentCandidate = Object.freeze({
      createdAtUnixMs: Date.parse(event.createdAt),
      event,
      expiresAtUnixMs: lease.value.effectiveExpiresAtUnixMs,
      status: lease.value.status,
    });
    const existing = candidatesByAgent.get(event.actor.agent.agentId);
    if (existing === undefined) {
      candidatesByAgent.set(event.actor.agent.agentId, [candidate]);
    } else {
      existing.push(candidate);
    }
  }

  const agents = [...candidatesByAgent.entries()]
    .flatMap(([agentId, candidates]) => aggregateAgent(agentId, candidates))
    .toSorted((left, right) => left.agentId.localeCompare(right.agentId));
  return Object.freeze({
    agents: Object.freeze(agents),
    joinedMemberIds: Object.freeze([...room.joinedMemberIds]),
    name: room.name,
    observedAtUnixMs,
    roomId: room.roomId,
    ...(room.topic === undefined ? {} : { topic: room.topic }),
  });
}

function aggregateAgent(
  agentId: string,
  candidates: readonly AgentCandidate[],
): readonly LobbyAgent[] {
  const matrixUserIds = new Set(
    candidates.map((candidate) => candidate.event.actor.agent.matrixUserId),
  );
  if (matrixUserIds.size !== 1) {
    return [];
  }
  const representative = candidates.toSorted(compareCandidates)[0];
  if (representative === undefined) {
    return [];
  }
  const event = representative.event;
  return [
    Object.freeze({
      agentId,
      ...(event.actor.agent.avatarUrl === undefined
        ? {}
        : { avatarUrl: event.actor.agent.avatarUrl }),
      displayName: event.actor.agent.displayName,
      instanceIds: Object.freeze(
        candidates.map((candidate) => candidate.event.actor.instanceId).toSorted(),
      ),
      matrixUserId: event.actor.agent.matrixUserId,
      status: representative.status,
      statusExpiresAtUnixMs: representative.expiresAtUnixMs,
      ...(event.visibility === 'detailed' && event.taskSummary !== undefined
        ? { summary: event.taskSummary }
        : {}),
      trust: 'unknown',
      visibility: event.visibility,
    }),
  ];
}

function compareCandidates(left: AgentCandidate, right: AgentCandidate): number {
  const priorityDifference = STATUS_PRIORITY[right.status] - STATUS_PRIORITY[left.status];
  if (priorityDifference !== 0) {
    return priorityDifference;
  }
  const timeDifference = right.createdAtUnixMs - left.createdAtUnixMs;
  return timeDifference === 0
    ? left.event.actor.instanceId.localeCompare(right.event.actor.instanceId)
    : timeDifference;
}

function limitProperties(limit: number) {
  return (value: object, context: z.core.$RefinementCtx<object>): void => {
    if (Object.keys(value).length > limit) {
      context.addIssue({ code: 'custom', message: `对象属性不得超过 ${String(limit)} 个。` });
    }
  };
}
