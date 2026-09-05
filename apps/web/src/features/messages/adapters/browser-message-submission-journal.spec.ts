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

it('加密准备结果跨实例恢复，聊天、提及和回复不会被日志丢弃', () => {
  window.sessionStorage.clear();
  const encryption = {
    algorithm: 'io.github.rainyflash.agentroom.content.aes-256-gcm.v1' as const,
    contextId: submissionId,
    keyBase64Url: 'BwcHBwcHBwcHBwcHBwcHBwcHBwcHBwcHBwcHBwcHBwc',
    nonceBase64Url: 'CQkJCQkJCQkJCQkJ',
    plaintextSizeBytes: 16000,
  };
  const prepared = {
    body: { bytes: new Uint8Array(16016).fill(1), digestSha256: 'a'.repeat(64) },
    encryption,
  };
  const journal = new BrowserMessageSubmissionJournal(window.sessionStorage);
  expect(journal.writeBody('scope', prepared).ok).toBe(true);
  const reference = { ...record().content, encryption, sizeBytes: 16016, mediaType: 'text/plain' };
  const value = {
    ...record(),
    content: reference,
    event: {
      ...record().event,
      content: { ...reference, fetchMode: 'on_demand' as const },
      preview: {
        ...record().event.preview,
        contentType: 'text/plain' as const,
        title: '😀'.repeat(120),
        summary: '😀'.repeat(500),
        conversation: { text: '😀'.repeat(4000), mentions: ['@agent:matrix.test'] },
      },
      relation: { kind: 'reply' as const, targetMessageId: submissionId },
    },
  };
  expect(journal.write(value).ok).toBe(true);
  const restored = new BrowserMessageSubmissionJournal(window.sessionStorage);
  expect(restored.readBody('scope')).toEqual({ ok: true, value: prepared });
  expect(restored.read(submissionId)).toEqual({ ok: true, value });
  restored.releaseBody('scope', submissionId);
  expect(new BrowserMessageSubmissionJournal(window.sessionStorage).readBody('scope')).toEqual({
    ok: true,
    value: null,
  });
  expect(new BrowserMessageSubmissionJournal(window.sessionStorage).read(submissionId)).toEqual({
    ok: true,
    value,
  });
});
