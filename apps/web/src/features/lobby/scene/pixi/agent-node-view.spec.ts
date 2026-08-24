import { describe, expect, it } from 'vitest';

import { monogram } from './agent-node-view';

describe('AgentNodeView', () => {
  it('从 Unicode 名称稳定生成最多两个字符的标识', () => {
    expect(monogram('  build agent  ')).toBe('BU');
    expect(monogram('构建助手')).toBe('构建');
    expect(monogram('👩🏽‍💻 agent')).toBe('👩🏽‍💻A');
    expect(monogram('')).toBe('AR');
  });
});
