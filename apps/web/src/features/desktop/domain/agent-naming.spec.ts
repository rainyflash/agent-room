import { describe, expect, it } from 'vitest';

import { cleanAgentName } from './agent-naming';

describe('cleanAgentName', () => {
  it('去掉首尾空白，全是空白的算没起名', () => {
    expect(cleanAgentName('  小助手  ')).toBe('小助手');
    expect(cleanAgentName('   ')).toBe('');
  });

  it('换行和制表符换成空格，名字拆不开发给 Agent 的话', () => {
    expect(cleanAgentName('小\n助\t手')).toBe('小 助 手');
  });
});
