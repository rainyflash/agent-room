// @vitest-environment jsdom

import { describe, expect, it } from 'vitest';

import { BrowserMessageSubmissionJournal } from './browser-message-submission-journal';
import type { MessageSubmissionRecord } from '@/features/messages/domain/publication';

const submissionId = '01990d9e-8400-7000-8000-000000000003';

describe('BrowserMessageSubmissionJournal', () => {
  it('跨适配器实例恢复不含正文的幂等提交记录', () => {
    window.sessionStorage.clear();
    const first = new BrowserMessageSubmissionJournal(window.sessionStorage);

    expect(first.write(record()).ok).toBe(true);

    const restored = new BrowserMessageSubmissionJournal(window.sessionStorage).read(submissionId);
    expect(restored.ok).toBe(true);
    if (restored.ok) {
      expect(restored.value).toEqual(record());
      expect(JSON.stringify(restored.value)).not.toContain('secret body');
      expect(Object.isFrozen(restored.value)).toBe(true);
    }
  });

  it('损坏的浏览器状态失败关闭而不是猜测提交结果', () => {
    window.sessionStorage.clear();
    window.sessionStorage.setItem(`agent-room.message-submission.v2.${submissionId}`, '{bad');

    expect(new BrowserMessageSubmissionJournal(window.sessionStorage).read(submissionId)).toEqual({
      error: { code: 'publication.persistence_failed', retryable: true },
      ok: false,
    });
  });
});

function record(): MessageSubmissionRecord {
  const content = {
    contentId: '01990d9e-8400-7000-8000-000000000004',
    digestSha256: 'a'.repeat(64),
    mediaType: 'text/markdown',
    sizeBytes: 10,
  } as const;
  return {
    content,
    event: {
      actor: {
        displayName: 'Rainy',
        kind: 'human',
        matrixUserId: '@rainy:agent-room.test',
        principalId: '01990d9e-8400-7000-8000-000000000001',
      },
      content: { ...content, fetchMode: 'on_demand' },
      correlationId: submissionId,
      createdAt: '2026-08-30T12:00:00.000Z',
      eventType: 'io.github.rainyflash.agentroom.message.preview.v2',
      id: submissionId,
      preview: {
        contentType: 'text/markdown',
        language: 'zh-CN',
        riskFlags: [],
        sensitivity: 'normal',
        summary: '摘要',
        title: '标题',
      },
      roomId: '!public:agent-room.test',
      schemaVersion: '2.0',
    },
    fingerprint: 'b'.repeat(64),
    roomId: '!public:agent-room.test',
    submissionId,
    transactionId: `agent-room-message-${submissionId}`,
  };
}
