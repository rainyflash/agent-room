import { useEffect, useRef, useState } from 'react';

import { useOptionalAppServices } from '@/app/app-services';
import { BrowserUuidV7Factory } from '@/shared/ids/browser-uuid-v7-factory';
import { enableInviteBackground } from '../application/enable-invite-background';
import type { AgentArrival } from '../domain/agent-arrivals';
import type { HostSessionDiagnostics } from '../domain/desktop-runtime';
import { offersInviteBackground, type InviteBackgroundState } from '../domain/invite-background';

const uuid = new BrowserUuidV7Factory();

/**
 * 勾着“你不在时也让它回话”时：这台电脑上新进来的 Agent 一登记这间房的后台回复，就替人打开一次。
 * 交回每个这样的 Agent（按到达的 key）此刻的样子：还没登记、正在打开、开了、没开成。
 * 已经在这台电脑上、换进这间房的 Agent 不管：它的后台回复登记在别的房间，这里开不了。
 */
export function useInviteBackground({
  active,
  sessions,
  arrivals,
  room,
  principalId,
}: {
  readonly active: boolean;
  readonly sessions: readonly HostSessionDiagnostics[] | null;
  readonly arrivals: readonly AgentArrival[];
  readonly room: { readonly roomId: string; readonly catalogId?: string } | null;
  readonly principalId: string | null;
}): ReadonlyMap<string, InviteBackgroundState> {
  const services = useOptionalAppServices();
  const [settled, setSettled] = useState<ReadonlyMap<string, InviteBackgroundState>>(
    () => new Map(),
  );
  const started = useRef(new Set<string>());
  const mounted = useRef(true);
  useEffect(() => {
    mounted.current = true;
    return () => {
      mounted.current = false;
    };
  }, []);

  const candidates = active && sessions !== null ? newArrivalSessions(arrivals, sessions) : [];
  useEffect(() => {
    if (services === null || principalId === null) return;
    for (const [key, session] of candidates) {
      if (started.current.has(key) || !offersInviteBackground(session, room)) continue;
      started.current.add(key);
      const settle = (state: InviteBackgroundState) => {
        if (!mounted.current) return;
        setSettled((current) => new Map(current).set(key, state));
      };
      settle({ kind: 'enabling' });
      void enableInviteBackground(
        { session, principalId, grantId: uuid.next() },
        { automation: services.automation, runtime: services.localRuntime },
      ).then((result) => {
        settle(result.ok ? { kind: 'on' } : { kind: 'failed', code: result.error.code });
      });
    }
  });

  const states = new Map<string, InviteBackgroundState>();
  for (const [key] of candidates) {
    states.set(key, settled.get(key) ?? { kind: 'waiting' });
  }
  return states;
}

/** 进了这间房、这台电脑上新来的 Agent 会话。 */
function newArrivalSessions(
  arrivals: readonly AgentArrival[],
  sessions: readonly HostSessionDiagnostics[],
): (readonly [string, HostSessionDiagnostics])[] {
  const found: (readonly [string, HostSessionDiagnostics])[] = [];
  for (const arrival of arrivals) {
    if (arrival.existing || arrival.elsewhere || arrival.state !== 'ready') continue;
    const session = sessions.find((entry) => entry.session.sessionId === arrival.key);
    if (session !== undefined) found.push([arrival.key, session]);
  }
  return found;
}
