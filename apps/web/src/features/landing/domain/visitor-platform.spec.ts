import { describe, expect, it } from 'vitest';

import { detectVisitorPlatform } from './visitor-platform';

describe('detectVisitorPlatform', () => {
  it('识别桌面系统，手机按 other 处理', () => {
    const cases: readonly (readonly [string, string])[] = [
      ['Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36', 'windows'],
      ['Mozilla/5.0 (Macintosh; Intel Mac OS X 10_15_7) AppleWebKit/605.1.15', 'macos'],
      ['Mozilla/5.0 (X11; Linux x86_64) AppleWebKit/537.36', 'linux'],
      ['Mozilla/5.0 (X11; CrOS x86_64 14541.0.0) AppleWebKit/537.36', 'linux'],
      // 手机浏览器没有桌面端可装，和未知系统一样走浏览器入口。
      ['Mozilla/5.0 (iPhone; CPU iPhone OS 17_0 like Mac OS X) AppleWebKit/605.1.15', 'other'],
      ['Mozilla/5.0 (Linux; Android 14) AppleWebKit/537.36', 'other'],
      // iPadOS 桌面模式自称 Macintosh，但仍然装不了桌面端。
      ['Mozilla/5.0 (iPad; CPU OS 17_0 like Mac OS X) AppleWebKit/605.1.15', 'other'],
      ['', 'other'],
    ];
    for (const [userAgent, expected] of cases) {
      expect(detectVisitorPlatform(userAgent), userAgent).toBe(expected);
    }
  });
});
