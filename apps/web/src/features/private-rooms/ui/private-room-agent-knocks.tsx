import { Banner, Button, Spinner, Toast } from '@agent-room/ui-system';
import { useMutation, useQueryClient } from '@tanstack/react-query';
import { Bot, Check, DoorOpen, X } from 'lucide-react';
import { useState } from 'react';
import { useTranslation } from 'react-i18next';

import {
  privateRoomAgentAccessQueryKey,
  privateRoomAgentKnocksQueryKey,
  usePrivateRoomAgentKnocks,
} from '@/features/private-rooms/data/private-room-queries';
import type {
  PrivateRoomAgentKnock,
  PrivateRoomFailure,
  PrivateRoomGateway,
} from '@/features/private-rooms/domain/private-room';
import { PrivateRoomFailureNotice } from '@/features/private-rooms/ui/private-room-failure-notice';
import { formatDateTime } from '@/shared/i18n/formatters';
import { ok, type Result } from '@/shared/result';

/** 接入对话框、房间设置开着时多久问一次有没有人敲门。 */
export const KNOCK_DIALOG_INTERVAL_MS = 3_000;
/** 房间页开着时多久问一次；页面在后台时不问，切回来马上问。 */
export const KNOCK_PAGE_INTERVAL_MS = 10_000;
/** 提示栈里最多同时挂几条敲门，多的合成一条。 */
const TOAST_LIMIT = 2;

type Answer = {
  readonly knock: PrivateRoomAgentKnock;
  readonly admit: boolean;
};

type Answered = {
  readonly agentId: string;
  readonly name: string;
  readonly admitted: boolean;
};

type KnockProblem =
  | { readonly kind: 'gone' }
  | { readonly kind: 'retry' }
  | { readonly kind: 'failed'; readonly failure: PrivateRoomFailure };

/**
 * 回答敲门（`specs/network-agents/knock.md`）：让它进来时服务器在这次请求里替它进房间，
 * 进去了才回答，所以按钮要转一会儿。回答过的从列表里拿掉；它已经不在敲门了也拿掉。
 */
function useKnockAnswers(rooms: PrivateRoomGateway, catalogId: string) {
  const queryClient = useQueryClient();
  const [answered, setAnswered] = useState<readonly Answered[]>([]);
  const [problem, setProblem] = useState<KnockProblem | null>(null);
  const knocksKey = privateRoomAgentKnocksQueryKey(catalogId);

  const forget = (agentId: string): void => {
    queryClient.setQueryData<Result<readonly PrivateRoomAgentKnock[], PrivateRoomFailure>>(
      knocksKey,
      (current) =>
        current?.ok === true
          ? ok(current.value.filter((knock) => knock.agentId !== agentId))
          : current,
    );
  };

  const mutation = useMutation({
    mutationFn: async ({ knock, admit }: Answer): Promise<Result<unknown, PrivateRoomFailure>> =>
      admit
        ? await rooms.admitKnock(catalogId, knock.agentId)
        : await rooms.declineKnock(catalogId, knock.agentId),
    onSuccess: async (result, { knock, admit }) => {
      if (!result.ok) {
        if (result.error.code === 'agent_knock.not_found') {
          forget(knock.agentId);
          setProblem({ kind: 'gone' });
        } else if (result.error.code === 'private_room.agent_entry_unavailable') {
          setProblem({ kind: 'retry' });
        } else {
          setProblem({ kind: 'failed', failure: result.error });
        }
        return;
      }
      setProblem(null);
      forget(knock.agentId);
      setAnswered((current) => [
        ...current.filter((entry) => entry.agentId !== knock.agentId),
        { agentId: knock.agentId, name: knock.displayName, admitted: admit },
      ]);
      await Promise.all([
        queryClient.invalidateQueries({ queryKey: knocksKey }),
        queryClient.invalidateQueries({ queryKey: privateRoomAgentAccessQueryKey(catalogId) }),
      ]);
    },
  });

  return {
    answer: (knock: PrivateRoomAgentKnock, admit: boolean): void => {
      setProblem(null);
      mutation.mutate({ knock, admit });
    },
    answered,
    pending: mutation.isPending ? mutation.variables : undefined,
    problem,
  };
}

export type PrivateRoomAgentKnocksProps = {
  readonly catalogId: string;
  readonly rooms: PrivateRoomGateway;
  /** 没人敲门时说什么；不给就什么都不显示。 */
  readonly emptyText?: string | undefined;
  /** 没人敲门时转个圈，表示正在等它敲门（接入对话框里）。 */
  readonly emptySpinner?: boolean;
  readonly intervalMs?: number;
};

/**
 * 在敲门的 Agent，每个带“让它进来”“不让进”。接入对话框和房间设置里用它；只有管理者能看到，
 * 读不到（不是管理者、旧版服务器）时什么都不显示。
 */
export function PrivateRoomAgentKnocks({
  catalogId,
  rooms,
  emptyText,
  emptySpinner = true,
  intervalMs = KNOCK_DIALOG_INTERVAL_MS,
}: PrivateRoomAgentKnocksProps) {
  const { i18n, t } = useTranslation();
  const knocks = usePrivateRoomAgentKnocks(rooms, catalogId, { intervalMs });
  const { answer, answered, pending, problem } = useKnockAnswers(rooms, catalogId);
  if (knocks.data?.ok !== true) return null;
  const waiting = knocks.data.value;

  return (
    <div className="agent-knocks">
      {waiting.length === 0 && answered.length === 0 && emptyText !== undefined ? (
        <p className="agent-knocks__empty">
          {emptySpinner ? <Spinner /> : null}
          {emptyText}
        </p>
      ) : null}
      {answered.length === 0 ? null : (
        <ul className="agent-knocks__answered">
          {answered.map((entry) => (
            <li key={entry.agentId}>
              {entry.admitted ? <Check aria-hidden="true" /> : <X aria-hidden="true" />}
              {t(entry.admitted ? 'privateRooms.knock.admitted' : 'privateRooms.knock.declined', {
                name: entry.name,
              })}
            </li>
          ))}
        </ul>
      )}
      {waiting.length === 0 ? null : (
        <>
          <ol aria-label={t('privateRooms.knock.title')} className="agent-knocks__list">
            {waiting.map((knock) => {
              const admitting = pending?.knock.agentId === knock.agentId && pending.admit;
              return (
                <li key={knock.agentId}>
                  <div className="private-room-member__identity">
                    <span>
                      <Bot aria-hidden="true" />
                    </span>
                    <div>
                      <strong>
                        {t('privateRooms.knock.knocking', { name: knock.displayName })}
                      </strong>
                      <small>
                        {admitting
                          ? t('privateRooms.knock.admitting', { name: knock.displayName })
                          : t('privateRooms.knock.network', {
                              time: formatDateTime(knock.knockedAtUnixMs, i18n.resolvedLanguage),
                            })}
                      </small>
                    </div>
                  </div>
                  <KnockActions
                    busy={pending !== undefined}
                    admitting={admitting}
                    knock={knock}
                    onAnswer={answer}
                  />
                </li>
              );
            })}
          </ol>
          <p className="agent-knocks__cost">{t('privateRooms.knock.cost')}</p>
        </>
      )}
      <KnockProblemNotice problem={problem} />
    </div>
  );
}

function KnockActions({
  admitting,
  busy,
  knock,
  onAnswer,
}: {
  readonly admitting: boolean;
  readonly busy: boolean;
  readonly knock: PrivateRoomAgentKnock;
  readonly onAnswer: (knock: PrivateRoomAgentKnock, admit: boolean) => void;
}) {
  const { t } = useTranslation();
  return (
    <div className="agent-knocks__actions">
      <Button
        aria-label={t('privateRooms.knock.admitAgent', { name: knock.displayName })}
        disabled={busy}
        icon={admitting ? <Spinner /> : <DoorOpen aria-hidden="true" />}
        onClick={() => {
          onAnswer(knock, true);
        }}
        size="compact"
        tone="primary"
        type="button"
      >
        {t('privateRooms.knock.admit')}
      </Button>
      <Button
        aria-label={t('privateRooms.knock.declineAgent', { name: knock.displayName })}
        disabled={busy}
        onClick={() => {
          onAnswer(knock, false);
        }}
        size="compact"
        tone="quiet"
        type="button"
      >
        {t('privateRooms.knock.decline')}
      </Button>
    </div>
  );
}

function KnockProblemNotice({ problem }: { readonly problem: KnockProblem | null }) {
  const { t } = useTranslation();
  if (problem === null) return null;
  if (problem.kind === 'failed') return <PrivateRoomFailureNotice failure={problem.failure} />;
  return (
    <Banner role="status" tone={problem.kind === 'gone' ? 'info' : 'warning'}>
      {t(problem.kind === 'gone' ? 'privateRooms.knock.gone' : 'privateRooms.knock.retry')}
    </Banner>
  );
}

/**
 * 房间页右下角的提示栈：管理者开着私人房间时，有 Agent 敲门就挂一条，能直接回答。
 * 同时最多两条，多的合成一条“还有 N 个在敲门”。
 */
export function PrivateRoomAgentKnockToasts({
  catalogId,
  rooms,
}: {
  readonly catalogId: string;
  readonly rooms: PrivateRoomGateway;
}) {
  const { t } = useTranslation();
  const knocks = usePrivateRoomAgentKnocks(rooms, catalogId, {
    intervalMs: KNOCK_PAGE_INTERVAL_MS,
  });
  const { answer, pending, problem } = useKnockAnswers(rooms, catalogId);
  if (knocks.data?.ok !== true) return null;
  const waiting = knocks.data.value;
  const shown = waiting.length > TOAST_LIMIT ? waiting.slice(0, TOAST_LIMIT - 1) : waiting;
  const more = waiting.length - shown.length;
  return (
    <>
      {shown.map((knock) => {
        const admitting = pending?.knock.agentId === knock.agentId && pending.admit;
        return (
          <Toast
            action={
              <KnockActions
                admitting={admitting}
                busy={pending !== undefined}
                knock={knock}
                onAnswer={answer}
              />
            }
            icon={<DoorOpen aria-hidden="true" />}
            key={knock.agentId}
            role="status"
            title={t('privateRooms.knock.toast.title', { name: knock.displayName })}
          >
            {admitting
              ? t('privateRooms.knock.admitting', { name: knock.displayName })
              : `${t('privateRooms.knock.toast.detail')} ${t('privateRooms.knock.cost')}`}
          </Toast>
        );
      })}
      {more > 0 ? (
        <Toast
          icon={<DoorOpen aria-hidden="true" />}
          role="status"
          title={t('privateRooms.knock.toast.more', { count: more })}
        >
          {t('privateRooms.knock.toast.moreDetail')}
        </Toast>
      ) : null}
      {problem === null || waiting.length === 0 ? null : (
        <Toast role="status" tone={problem.kind === 'gone' ? 'info' : 'warning'}>
          {problem.kind === 'failed'
            ? t('privateRooms.failure.title')
            : t(problem.kind === 'gone' ? 'privateRooms.knock.gone' : 'privateRooms.knock.retry')}
        </Toast>
      )}
    </>
  );
}
