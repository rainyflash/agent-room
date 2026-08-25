import { describe, expect, it } from 'vitest';

import { canonicalMessageKey, validateCatalogContract } from './catalog-contract';
import { resources } from './resources';

describe('类型化消息目录契约', () => {
  it('允许不同语言采用不同 CLDR 复数分支，但要求基础键与插值参数一致', () => {
    const failures = validateCatalogContract('en', resources.en.translation, {
      'zh-CN': resources['zh-CN'].translation,
    });

    expect(failures).toEqual([]);
    expect(canonicalMessageKey('lobby.room.agents_one')).toBe('lobby.room.agents');
  });

  it('缺键与占位符漂移会明确失败', () => {
    const failures = validateCatalogContract(
      'en',
      { greeting: 'Hello {{name}}', idle: 'Idle' },
      {
        broken: { greeting: '你好 {{operator}}' },
      },
    );

    expect(failures).toEqual([
      {
        code: 'i18n.placeholder_mismatch',
        detail: 'name != operator',
        key: 'greeting',
        language: 'broken',
      },
      {
        code: 'i18n.missing_canonical_key',
        detail: 'en',
        key: 'idle',
        language: 'broken',
      },
    ]);
  });
});
