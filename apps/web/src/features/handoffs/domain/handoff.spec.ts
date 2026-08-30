import { describe, expect, it } from 'vitest';

import {
  type HandoffApprovalRequest,
  validateHandoffApproval,
} from '@/features/handoffs/domain/handoff';

describe('上下文交付授权', () => {
  it('接受精确目标、文本范围、用途和一小时内期限', () => {
    expect(validateHandoffApproval(request(), 1_000)).toEqual([]);
  });

  it('拒绝空范围、重复范围以及与媒体类型不一致的范围', () => {
    expect(validateHandoffApproval({ ...request(), permissions: [] }, 1_000)).toContain(
      'content_scope_invalid',
    );
    expect(
      validateHandoffApproval({ ...request(), permissions: ['read_text', 'read_text'] }, 1_000),
    ).toContain('content_scope_invalid');
    expect(
      validateHandoffApproval(
        {
          ...request(),
          permissions: ['read_attachments'],
          source: {
            ...request().source,
            content: { ...request().source.content, mediaType: 'text/plain' },
          },
        },
        1_000,
      ),
    ).toContain('content_scope_invalid');
  });

  it('拒绝错实例格式、篡改摘要和过期或超长授权', () => {
    expect(
      validateHandoffApproval(
        { ...request(), target: { ...request().target, instanceId: 'not-an-instance' } },
        1_000,
      ),
    ).toContain('target_invalid');
    expect(
      validateHandoffApproval(
        {
          ...request(),
          source: {
            ...request().source,
            content: { ...request().source.content, digestSha256: 'tampered' },
          },
        },
        1_000,
      ),
    ).toContain('source_invalid');
    expect(validateHandoffApproval({ ...request(), expiresAtUnixMs: 1_000 }, 1_000)).toContain(
      'expiry_invalid',
    );
    expect(validateHandoffApproval({ ...request(), expiresAtUnixMs: 3_601_001 }, 1_000)).toContain(
      'expiry_invalid',
    );
  });
});

function request(): HandoffApprovalRequest {
  return {
    expiresAtUnixMs: 901_000,
    handoffId: '01990d9e-8400-7000-8000-000000000010',
    permissions: ['read_text', 'include_metadata'],
    purpose: 'summarize',
    source: {
      actor: {
        agentId: '01990d9e-8400-7000-8000-000000000001',
        displayName: 'Remote Agent',
        instanceId: '01990d9e-8400-7000-8000-000000000002',
        kind: 'agent',
        matrixUserId: '@remote:agent-room.test',
        provenance: 'autonomous_agent',
      },
      content: {
        contentId: '01990d9e-8400-7000-8000-000000000003',
        digestSha256: 'ab'.repeat(32),
        mediaType: 'text/plain',
        sizeBytes: 128,
      },
      matrixEventId: '$source:agent-room.test',
      messageId: '01990d9e-8400-7000-8000-000000000004',
      riskFlags: ['untrusted_instructions'],
      roomId: '!builders:agent-room.test',
    },
    target: {
      agentId: '01990d9e-8400-7000-8000-000000000005',
      displayName: 'Local Codex',
      instanceId: '01990d9e-8400-7000-8000-000000000006',
    },
  };
}
