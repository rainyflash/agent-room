import { describe, expect, it } from 'vitest';

import {
  orderSignalProjections,
  type SignalKind,
  type SignalProjection,
} from '@/features/signals/domain/signal';

describe('orderSignalProjections', () => {
  it('按安全来源与风险确定性排序，且不修改输入', () => {
    const input = [signal('room_message', 30), signal('mention', 10), signal('sync_issue', 1)];

    const ordered = orderSignalProjections(input);

    expect(ordered.map((item) => item.kind)).toEqual(['sync_issue', 'mention', 'room_message']);
    expect(input.map((item) => item.kind)).toEqual(['room_message', 'mention', 'sync_issue']);
    expect(Object.isFrozen(ordered)).toBe(true);
  });

  it('同一来源先展示风险更高的信号，再按时间和稳定标识排序', () => {
    const ordered = orderSignalProjections([
      signal('room_message', 20, []),
      signal('room_message', 10, ['external_link']),
      signal('room_message', 20, [], 'z'),
    ]);

    expect(ordered.map((item) => item.signalId)).toEqual([
      'room_message-10-external_link',
      'room_message-20--z',
      'room_message-20-',
    ]);
  });
});

function signal(
  kind: SignalKind,
  occurredAtUnixMs: number,
  riskFlags: readonly string[] = [],
  suffix = '',
): SignalProjection {
  return {
    action: { kind: 'retry_sync' },
    actor: null,
    edited: false,
    kind,
    lifecycle: 'active',
    occurredAtUnixMs,
    riskFlags,
    signalId: `${kind}-${String(occurredAtUnixMs)}-${riskFlags.join('-')}${suffix.length === 0 ? '' : `-${suffix}`}`,
    summary: 'Summary',
    title: 'Title',
  };
}
