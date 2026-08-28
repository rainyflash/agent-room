import { describe, expect, it } from 'vitest';

import { bypassesNavigationFallback } from './navigation-fallback';

describe('PWA 导航回退排除规则', () => {
  it.each([
    '/connect/finalize',
    '/connect/finalize?state=opaque&session_state=opaque&code=opaque',
    '/_agent-room/api/auth/session',
    '/_agent-room/api?probe=1',
    '/_agent-room/healthz',
    '/_agent-room/healthz?probe=1',
  ])('让服务端处理 %s', (pathnameAndSearch) => {
    expect(bypassesNavigationFallback(pathnameAndSearch)).toBe(true);
  });

  it.each(['/connect', '/connect/finalize/extra', '/lobby', '/rooms/private'])(
    '继续由单页应用处理 %s',
    (pathnameAndSearch) => {
      expect(bypassesNavigationFallback(pathnameAndSearch)).toBe(false);
    },
  );
});
