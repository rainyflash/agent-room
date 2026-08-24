import { describe, expect, it } from 'vitest';

import {
  inspectPublicationRisks,
  type MessagePublicationDraft,
  validatePublicationDraft,
} from './publication';

describe('消息发布意图', () => {
  it('按 Unicode 字符和 UTF-8 字节执行协议边界', () => {
    expect(validatePublicationDraft(draft({ title: '界'.repeat(120) }))).toEqual([]);
    expect(validatePublicationDraft(draft({ title: '界'.repeat(121) }))).toContain(
      'title_invalid',
    );
    expect(validatePublicationDraft(draft({ body: '' }))).toContain('body_empty');
  });

  it('拒绝重复、超限或非协议格式的风险标签', () => {
    expect(
      validatePublicationDraft(
        draft({ riskFlags: ['external_links', 'external_links', 'Bad-Flag'] }),
      ),
    ).toContain('risk_flags_invalid');
  });

  it('只把外部链接和 HTML 标记为风险，不改写正文', () => {
    const body = 'Review https://example.com and <script>alert(1)</script> as inert text.';

    expect(inspectPublicationRisks(body)).toEqual(['external_links', 'html_markup']);
    expect(body).toContain('<script>');
  });
});

function draft(overrides: Partial<MessagePublicationDraft> = {}): MessagePublicationDraft {
  return {
    body: 'A verified body.',
    language: 'en',
    mediaType: 'text/markdown',
    riskFlags: [],
    sensitivity: 'normal',
    summary: 'A bounded summary.',
    title: 'Build result',
    ...overrides,
  };
}
