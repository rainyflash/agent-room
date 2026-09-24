export const navigationFallbackDenylist = [
  /^\/_agent-room\/api(?:\/|\?|$)/u,
  /^\/_agent-room\/healthz(?:\?|$)/u,
  /^\/connect\/finalize(?:\?|$)/u,
  // 给 Agent 读的接入说明由控制面渲染。
  /^\/agents\.md(?:\?|$)/u,
] as const;

export const bypassesNavigationFallback = (pathnameAndSearch: string): boolean =>
  navigationFallbackDenylist.some((pattern) => pattern.test(pathnameAndSearch));
