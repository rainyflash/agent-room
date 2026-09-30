import type { TFunction } from 'i18next';
import { Button, Spinner } from '@agent-room/ui-system';
import { AlertTriangle, CircleCheckBig, LogOut } from 'lucide-react';
import { useEffect, useState, type ReactNode } from 'react';
import { useTranslation } from 'react-i18next';

import type { MessageGateway } from '@/features/messages/domain/message';
import type { AgentArrival } from '../domain/agent-arrivals';
import { InviteReplyProgress } from './invite-reply-progress';

/** 复制后这么久还没人进来，提示去问问 Agent。 */
const SLOW_HINT_MS = 45_000;

type FirstReply = {
  readonly gateway: MessageGateway;
  readonly principalId: string;
  readonly roomId: string;
  readonly startedAt: number;
};

/**
 * 对话框底部：刚进来的 Agent 一个个列出来，不用去别处看。还没人进来时说清接下来会怎样；
 * 看不到的情况（网络方式又没有房间）如实说它会去哪。
 */
export function AgentArrivalsPanel({
  arrivals,
  copiedAt,
  observable,
  unavailable,
  firstReply,
  startConversation,
  onDone,
}: {
  readonly arrivals: readonly AgentArrival[];
  readonly copiedAt: number | null;
  /** 能不能看到它进来：网络方式又不在房间里时看不到。 */
  readonly observable: boolean;
  /** 读不到这台电脑上的 Agent 会话。 */
  readonly unavailable: boolean;
  readonly firstReply: FirstReply | null;
  readonly startConversation: boolean;
  readonly onDone: () => void;
}) {
  const { t } = useTranslation();
  const slow = useSlowHint(copiedAt, arrivals.length === 0);
  const firstReady = arrivals.find(
    (arrival) => arrival.state === 'ready' && !arrival.elsewhere && arrival.agentId !== null,
  );

  if (arrivals.length === 0) {
    return (
      <section aria-live="polite" className="agent-invite__arrivals" data-state="empty">
        {!observable ? (
          <p>{t('agentInvite.arrival.lobby')}</p>
        ) : copiedAt === null ? (
          <p>{t('agentInvite.arrival.idle')}</p>
        ) : (
          <p className="agent-invite__waiting">
            <Spinner />
            {t(slow ? 'agentInvite.arrival.slow' : 'agentInvite.arrival.waiting')}
          </p>
        )}
        {unavailable ? (
          <p className="agent-invite__note">{t('agentInvite.arrival.unavailable')}</p>
        ) : null}
      </section>
    );
  }

  return (
    <section aria-live="polite" className="agent-invite__arrivals" data-state="arrived">
      <ul>
        {arrivals.map((arrival) => (
          <li data-state={arrival.state} key={arrival.key}>
            <ArrivalIcon arrival={arrival} />
            <div>
              <strong>{arrivalTitle(t, arrival)}</strong>
              {arrival.state === 'ready' && arrival.active && !arrival.elsewhere ? (
                <span>{t('agentInvite.arrival.active')}</span>
              ) : null}
              {arrival.state === 'failed' && arrival.errorCode !== null ? (
                <code>{t('agentInvite.arrival.code', { code: arrival.errorCode })}</code>
              ) : null}
              {firstReply !== null &&
              firstReady?.key === arrival.key &&
              firstReady.agentId !== null ? (
                <InviteReplyProgress
                  agentId={firstReady.agentId}
                  gateway={firstReply.gateway}
                  principalId={firstReply.principalId}
                  roomId={firstReply.roomId}
                  startedAt={firstReply.startedAt}
                />
              ) : null}
            </div>
          </li>
        ))}
      </ul>
      {firstReady === undefined ? null : (
        <Button onClick={onDone} tone="primary">
          {t(startConversation ? 'agentInvite.arrival.chat' : 'agentInvite.done')}
        </Button>
      )}
    </section>
  );
}

function ArrivalIcon({ arrival }: { readonly arrival: AgentArrival }): ReactNode {
  switch (arrival.state) {
    case 'starting':
      return <Spinner />;
    case 'ready':
      return <CircleCheckBig aria-hidden="true" className="agent-invite__arrival-icon" />;
    case 'failed':
      return <AlertTriangle aria-hidden="true" className="agent-invite__arrival-icon" />;
    case 'closed':
      return <LogOut aria-hidden="true" className="agent-invite__arrival-icon" />;
  }
}

function arrivalTitle(t: TFunction, arrival: AgentArrival): string {
  const name = arrival.displayName;
  switch (arrival.state) {
    case 'starting':
      return t('agentInvite.arrival.starting', { name });
    case 'ready':
      return arrival.elsewhere
        ? t('agentInvite.arrival.elsewhere', { name })
        : t('agentInvite.arrival.ready', { name });
    case 'failed':
      return t('agentInvite.arrival.failed', { name });
    case 'closed':
      return t('agentInvite.arrival.closed', { name });
  }
}

/** 复制后等太久还没人进来：换一句提示。 */
function useSlowHint(copiedAt: number | null, waiting: boolean): boolean {
  const [slowFor, setSlowFor] = useState<number | null>(null);
  useEffect(() => {
    if (copiedAt === null || !waiting) return undefined;
    const timer = setTimeout(
      () => {
        setSlowFor(copiedAt);
      },
      Math.max(0, copiedAt + SLOW_HINT_MS - Date.now()),
    );
    return () => {
      clearTimeout(timer);
    };
  }, [copiedAt, waiting]);
  return copiedAt !== null && slowFor === copiedAt;
}
