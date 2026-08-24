import { describe, expect, it } from 'vitest';

import { effectiveLanguage, readLanguagePreference, resolveSystemLanguage } from './i18n';

describe('语言偏好', () => {
  it('优先匹配中文系统语言并对未知语言回退英文', () => {
    expect(resolveSystemLanguage(['fr-FR', 'zh-Hans-CN'])).toBe('zh-CN');
    expect(resolveSystemLanguage(['fr-FR'])).toBe('en');
    expect(effectiveLanguage('en', ['zh-CN'])).toBe('en');
  });

  it('存储不可用时回退到系统偏好', () => {
    const storage = {
      getItem: () => {
        throw new Error('blocked');
      },
    } as Pick<Storage, 'getItem'> as Storage;

    expect(readLanguagePreference(storage)).toBe('system');
  });
});
