import { describe, expect, it } from 'vitest';

import { BrowserSubmissionIdFactory } from './browser-submission-id-factory';

describe('浏览器消息提交标识工厂', () => {
  it('生成携带毫秒时间且符合变体位的 UUIDv7', () => {
    const timestamp = 1_790_000_000_123;
    const factory = new BrowserSubmissionIdFactory(
      () => timestamp,
      (bytes) => bytes.fill(0xab),
    );

    const id = factory.next();

    expect(id).toMatch(
      /^[0-9a-f]{8}-[0-9a-f]{4}-7[0-9a-f]{3}-[89ab][0-9a-f]{3}-[0-9a-f]{12}$/u,
    );
    expect(Number.parseInt(id.replaceAll('-', '').slice(0, 12), 16)).toBe(timestamp);
  });
});
