import { describe, expect, it, vi } from 'vitest';

import { WebObserverMessagePublisher } from './web-observer-message-publisher';

describe('纯 Web 消息发布边界', () => {
  it('明确报告缺少 Agent 实例身份，不伪造签名或调用网络', async () => {
    const publisher = new WebObserverMessagePublisher();

    await expect(publisher.resolveIdentity()).resolves.toEqual({
      error: { code: 'publication.bridge_unavailable', retryable: false },
      ok: false,
    });
    await expect(
      publisher.publish(
        {
          body: 'body',
          language: 'en',
          mediaType: 'text/plain',
          riskFlags: [],
          roomId: '!room:agent-room.test',
          sensitivity: 'normal',
          submissionId: '01990d9e-8400-7000-8000-000000000003',
          summary: 'summary',
          title: 'title',
        },
        vi.fn(),
      ),
    ).resolves.toEqual({
      error: { code: 'publication.bridge_unavailable', retryable: false },
      ok: false,
    });
  });
});
