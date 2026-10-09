import { describe, expect, it } from 'vitest';
import fixture from '../fixtures/agent-lifecycle.json' with { type: 'json' };
import {
  agentLifecyclePolicy,
  evaluateAgentLifecycle,
  projectAgentLifecycles,
  type AgentPresenceEvidence,
  type MatrixPresenceState,
} from './agent-lifecycle.js';

type PresenceCase = {
  readonly state: string;
  readonly offlineSeenAt: number | null;
  readonly lastActive: number | null;
} | null;

/** 夹具里的名片用例：`liveness` 和它拿到的在线状态。 */
function cardEvidence(entry: object): Partial<AgentPresenceEvidence> {
  if (!('liveness' in entry) || entry.liveness !== 'presence') return {};
  const presence = ('presence' in entry ? entry.presence : null) as PresenceCase;
  return {
    liveness: 'presence',
    ...(presence === null
      ? {}
      : {
          presence: {
            state: presence.state as MatrixPresenceState,
            ...(presence.offlineSeenAt === null
              ? {}
              : { offlineSeenAtUnixMs: presence.offlineSeenAt }),
            ...(presence.lastActive === null ? {} : { lastActiveAtUnixMs: presence.lastActive }),
          },
        }),
  };
}

describe('shared agent lifecycle contract', () => {
  it('matches the cross-language policy', () => {
    expect(agentLifecyclePolicy).toMatchObject(fixture.policy);
  });
  it.each(fixture.cases)('$name', (entry) => {
    const result = evaluateAgentLifecycle(
      {
        agentId: 'agent',
        reportedStatus: entry.status,
        lastActiveAtUnixMs: entry.lastActive,
        leaseExpiresAtUnixMs: entry.expires,
        ...(entry.polled === null ? {} : { lastPolledAtUnixMs: entry.polled }),
        ...('legacy' in entry && entry.legacy
          ? {}
          : { listeningUntilUnixMs: entry.listeningUntil }),
        ...cardEvidence(entry),
      },
      entry.now,
    );
    expect(result).toEqual({
      connection: entry.connection,
      reception: entry.reception,
      offlineSinceUnixMs: entry.offlineSince,
      archived: entry.archived,
      archiveReason: entry.archived ? 'expired' : null,
    });
  });
  it('bounds offline members without archiving connected agents, even at 1,000 identities', () => {
    const agents: AgentPresenceEvidence[] = Array.from({ length: 1000 }, (_, id) => ({
      agentId: String(id).padStart(4, '0'),
      reportedStatus: 'offline',
      lastActiveAtUnixMs: 10_000 + id,
      leaseExpiresAtUnixMs: 20_000,
    }));
    agents.push({
      agentId: 'online',
      reportedStatus: 'completed',
      lastActiveAtUnixMs: 0,
      leaseExpiresAtUnixMs: 200_000,
    });
    const projected = projectAgentLifecycles(agents, 100_000);
    expect([...projected.values()].filter((state) => !state.archived)).toHaveLength(101);
    expect(projected.get('0000')?.archiveReason).toBe('capacity');
    expect(projected.get('0999')?.archived).toBe(false);
    expect(projected.get('online')?.connection).toBe('online');
  });
  it('applies the room policy without resetting offline age', () => {
    const evidence = {
      agentId: 'agent',
      reportedStatus: 'offline',
      lastActiveAtUnixMs: 10_000,
      leaseExpiresAtUnixMs: 20_000,
    };
    expect(evaluateAgentLifecycle(evidence, 2 * 86_400_000, 1).archived).toBe(true);
    expect(evaluateAgentLifecycle(evidence, 2 * 86_400_000, 7).archived).toBe(false);
    expect(evaluateAgentLifecycle(evidence, 8 * 86_400_000, 30).archived).toBe(false);
  });
});
