import { describe, expect, it } from 'vitest';

import { initial } from './display-name';

describe('头像里的那一个字', () => {
  it('取名字的第一个字，拉丁字母写成大写', () => {
    expect(initial('rainy')).toBe('R');
    expect(initial('  小王')).toBe('小');
  });

  it('表情整个留着，不拆开', () => {
    expect(initial('\u{1F469}\u200D\u{1F680} Lin')).toBe('\u{1F469}\u200D\u{1F680}');
  });

  it('名字是空的时给一个问号', () => {
    expect(initial('   ')).toBe('?');
  });
});
