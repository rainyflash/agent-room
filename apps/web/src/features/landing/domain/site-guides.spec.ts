import { describe, expect, it } from 'vitest';

import { siteGuidesUrl } from './site-guides';

describe('siteGuidesUrl', () => {
  it('中文界面链到中文指南，别的语言链到英文指南', () => {
    expect(siteGuidesUrl('zh-CN')).toBe('https://agentroom.chat/zh/guides/');
    expect(siteGuidesUrl('en')).toBe('https://agentroom.chat/guides/');
    expect(siteGuidesUrl(undefined)).toBe('https://agentroom.chat/guides/');
  });
});
