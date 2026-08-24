import type { AgentStatusEvent } from '../../protocol-types/src/generated.js';
import { describe, expect, test } from 'vitest';

import { evaluateAgentStatusLease } from './status-lease.js';

const policy = {
  maximumLeaseMs: 300_000,
  allowedClockSkewMs: 30_000,
} as const;

describe('Agent 状态租约评估', () => {
  test('Bridge 崩溃后无需清理事件也会在本地转为离线', () => {
    const event = statusEvent('2026-08-24T12:00:00.000Z', '2026-08-24T12:05:00.000Z');
    const observedAt = Date.parse('2026-08-24T12:00:01.000Z');

    const active = evaluateAgentStatusLease(
      event,
      observedAt,
      Date.parse('2026-08-24T12:05:29.999Z'),
      policy,
    );
    const expired = evaluateAgentStatusLease(
      event,
      observedAt,
      Date.parse('2026-08-24T12:05:30.000Z'),
      policy,
    );

    expect(active).toMatchObject({ ok: true, value: { status: 'working' } });
    expect(expired).toMatchObject({ ok: true, value: { status: 'offline' } });
  });

  test('允许有限发送端时钟偏差但使用本地观测时间封顶', () => {
    const behind = statusEvent('2026-08-24T11:59:30.000Z', '2026-08-24T12:04:30.000Z');
    const observedAt = Date.parse('2026-08-24T12:00:00.000Z');
    const tolerated = evaluateAgentStatusLease(
      behind,
      observedAt,
      Date.parse('2026-08-24T12:04:59.999Z'),
      policy,
    );

    expect(tolerated).toMatchObject({ ok: true, value: { status: 'working' } });

    const ahead = statusEvent('2026-08-24T12:00:30.000Z', '2026-08-24T12:05:30.000Z');
    const capped = evaluateAgentStatusLease(
      ahead,
      observedAt,
      Date.parse('2026-08-24T12:05:30.000Z'),
      policy,
    );
    expect(capped).toEqual({
      ok: true,
      value: {
        status: 'offline',
        effectiveExpiresAtUnixMs: Date.parse('2026-08-24T12:05:30.000Z'),
      },
    });
  });

  test('拒绝超长租约与超出容忍范围的未来事件', () => {
    const observedAt = Date.parse('2026-08-24T12:00:00.000Z');
    expect(
      evaluateAgentStatusLease(
        statusEvent('2026-08-24T12:00:00.000Z', '2026-08-24T12:10:00.000Z'),
        observedAt,
        observedAt,
        policy,
      ),
    ).toEqual({ ok: false, error: 'invalid_lease_duration' });
    expect(
      evaluateAgentStatusLease(
        statusEvent('2026-08-24T12:00:31.000Z', '2026-08-24T12:05:31.000Z'),
        observedAt,
        observedAt,
        policy,
      ),
    ).toEqual({ ok: false, error: 'future_event' });
  });

  test('显式离线事件不会被有效租期重新解释为在线', () => {
    const event = {
      ...statusEvent('2026-08-24T12:00:00.000Z', '2026-08-24T12:05:00.000Z'),
      status: 'offline',
    } satisfies AgentStatusEvent;
    const observedAt = Date.parse('2026-08-24T12:00:01.000Z');

    expect(evaluateAgentStatusLease(event, observedAt, observedAt, policy)).toMatchObject({
      ok: true,
      value: { status: 'offline' },
    });
  });
});

function statusEvent(createdAt: string, leaseExpiresAt: string): AgentStatusEvent {
  return {
    schemaVersion: '1.0',
    eventType: 'org.agentroom.agent.status.v1',
    id: '01945c1e-7b5a-7c7f-8a28-2de53f56a9a7',
    createdAt,
    actor: {
      agent: {
        agentId: '01945c1e-7b5a-7c7f-8a28-2de53f56a9a3',
        displayName: '构建助手',
        matrixUserId: '@build-agent:matrix.test',
      },
      instanceId: '01945c1e-7b5a-7c7f-8a28-2de53f56a9a4',
      provenance: 'autonomous_agent',
    },
    correlationId: '01945c1e-7b5a-7c7f-8a28-2de53f56a9a8',
    signature: 'BBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBB',
    status: 'working',
    visibility: 'coarse',
    leaseExpiresAt,
  };
}
