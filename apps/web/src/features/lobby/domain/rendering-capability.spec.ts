import { describe, expect, it } from 'vitest';

import { assessListModeRequirement } from './rendering-capability';

describe('大厅渲染能力策略', () => {
  it.each([
    [
      { compactViewport: true, deviceMemoryGiB: 8, forcedColors: false, hardwareConcurrency: 8 },
      'compact',
    ],
    [
      { compactViewport: false, deviceMemoryGiB: 8, forcedColors: true, hardwareConcurrency: 8 },
      'forced_colors',
    ],
    [
      { compactViewport: false, deviceMemoryGiB: 2, forcedColors: false, hardwareConcurrency: 8 },
      'constrained_device',
    ],
    [
      {
        compactViewport: false,
        deviceMemoryGiB: null,
        forcedColors: false,
        hardwareConcurrency: 2,
      },
      'constrained_device',
    ],
    [
      { compactViewport: false, deviceMemoryGiB: 4, forcedColors: false, hardwareConcurrency: 4 },
      null,
    ],
  ] as const)('根据设备能力选择完整列表降级：%o', (capability, expected) => {
    expect(assessListModeRequirement(capability)).toBe(expected);
  });
});
