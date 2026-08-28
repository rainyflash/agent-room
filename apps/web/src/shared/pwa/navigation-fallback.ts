export const navigationFallbackDenylist = [
  /^\/_agent-room\/api(?:\/|\?|$)/u,
  /^\/_agent-room\/healthz(?:\?|$)/u,
  /^\/connect\/finalize(?:\?|$)/u,
] as const;

export const bypassesNavigationFallback = (pathnameAndSearch: string): boolean =>
  navigationFallbackDenylist.some((pattern) => pattern.test(pathnameAndSearch));
