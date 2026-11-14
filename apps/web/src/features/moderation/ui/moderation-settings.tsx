import { Button } from '@agent-room/ui-system';
import { CircleAlert, LoaderCircle, RefreshCw } from 'lucide-react';
import { useMutation, useQueryClient } from '@tanstack/react-query';
import { useEffect, useMemo, useState } from 'react';
import { useTranslation } from 'react-i18next';

import {
  moderationActionListQueryKey,
  moderationAuditListQueryKey,
  moderationRoomCaseListQueryKey,
  useAgentModerationLedger,
  useModerationActions,
  useModerationAudit,
  useModerationCapabilities,
  useModerationRoomCases,
} from '@/features/moderation/data/moderation-queries';
import {
  nextModerationExpiry,
  type ApplyModerationActionInput,
  type ModerationAction,
  type ModerationFailure,
  type ModerationGateway,
} from '@/features/moderation/domain/moderation';
import {
  AgentModerationSettings,
  type AgentModerationSettingsProps,
} from '@/features/moderation/ui/agent-moderation-settings';
import { ModerationActionForm } from '@/features/moderation/ui/moderation-action-form';
import {
  ModerationActionLedger,
  ModerationAuditLedger,
  ModerationCaseLedger,
} from '@/features/moderation/ui/moderation-ledger';
import { BrowserUuidV7Factory } from '@/shared/ids/browser-uuid-v7-factory';

/** “交给 Agent 管”要的那几样；只有私人房间的房主和管理员有。 */
export type ModerationAgentsProps = Omit<
  AgentModerationSettingsProps,
  'actions' | 'catalogId' | 'onReauthenticate' | 'recentlyAuthenticated'
>;

export type ModerationSettingsProps = {
  /** 给了就有“交给 Agent 管”，台账里写哪条是哪个 Agent 照规则做的。 */
  readonly agents?: ModerationAgentsProps | undefined;
  readonly catalogId: string;
  readonly gateway: ModerationGateway;
  readonly onReauthenticate: () => void;
  readonly recentlyAuthenticated: boolean;
};

type ModerationCommand =
  | { readonly input: ApplyModerationActionInput; readonly kind: 'apply' }
  | { readonly actionId: string; readonly kind: 'reverse' };

/** 浏览器定时器最长能等多久；再长就先醒一次、重新算。 */
const MAXIMUM_TIMER_MS = 2_147_483_647;

/**
 * 房间设置里的“治理”：举报、处理动作和审计记录。只有房间管理者（或能看审计的人）看得到这一节，
 * 没有权限时不去读任何受限资源。
 */
export function ModerationSettings({
  agents,
  catalogId,
  gateway,
  onReauthenticate,
  recentlyAuthenticated,
}: ModerationSettingsProps) {
  const queryClient = useQueryClient();
  const identifiers = useMemo(() => new BrowserUuidV7Factory(), []);
  const capabilitiesQuery = useModerationCapabilities(gateway, catalogId);
  const capabilities = capabilitiesQuery.data?.ok === true ? capabilitiesQuery.data.value : null;
  const canModerateRoom = capabilities?.canModerateRoom === true;
  const canReadAudit = capabilities?.canReadAudit === true;
  const casesQuery = useModerationRoomCases(gateway, catalogId, canModerateRoom);
  const actionsQuery = useModerationActions(gateway, catalogId, canModerateRoom);
  const auditQuery = useModerationAudit(gateway, catalogId, canReadAudit);
  const agentLedgerQuery = useAgentModerationLedger(agents?.gateway, catalogId, canModerateRoom);
  const cases = casesQuery.data?.ok === true ? casesQuery.data.value : null;
  const actions = actionsQuery.data?.ok === true ? actionsQuery.data.value : null;
  const audit = auditQuery.data?.ok === true ? auditQuery.data.value : null;
  const agentLedger = agentLedgerQuery.data?.ok === true ? agentLedgerQuery.data.value : null;
  const now = useLedgerClock(actions);
  const hasVisibleData = cases !== null || actions !== null || audit !== null;
  const mutation = useMutation({
    mutationFn: async (command: ModerationCommand) =>
      command.kind === 'apply'
        ? await gateway.applyAction(identifiers.next(), catalogId, command.input)
        : await gateway.reverseAction(command.actionId),
    onSuccess: async (result) => {
      if (!result.ok) {
        return;
      }
      await Promise.all([
        queryClient.invalidateQueries({ queryKey: moderationActionListQueryKey(catalogId) }),
        queryClient.invalidateQueries({ queryKey: moderationRoomCaseListQueryKey(catalogId) }),
        queryClient.invalidateQueries({ queryKey: moderationAuditListQueryKey(catalogId) }),
      ]);
    },
  });
  const failure = mutation.data?.ok === false ? mutation.data.error : null;
  const command = mutation.variables;
  const reversingMute =
    command?.kind === 'reverse' &&
    actions?.some((action) => action.actionId === command.actionId && action.kind === 'mute') ===
      true;
  const pendingActionId =
    mutation.isPending && mutation.variables.kind === 'reverse'
      ? mutation.variables.actionId
      : null;

  if (!canModerateRoom && !canReadAudit) {
    return null;
  }

  const retry = async (): Promise<void> => {
    await Promise.all([
      capabilitiesQuery.refetch(),
      ...(canModerateRoom ? [casesQuery.refetch(), actionsQuery.refetch()] : []),
      ...(canReadAudit ? [auditQuery.refetch()] : []),
    ]);
  };

  return (
    <div className="moderation-settings">
      {failure === null ? null : (
        <ModerationFailureNotice
          failure={failure}
          onReauthenticate={onReauthenticate}
          reversingMute={reversingMute}
        />
      )}
      {!hasVisibleData ? (
        <ModerationBoundary onRetry={retry} />
      ) : (
        <div className="moderation-governance__workspace">
          <div className="moderation-governance__primary">
            {cases === null ? null : <ModerationCaseLedger cases={cases} />}
            {actions === null ? null : (
              <ModerationActionForm
                cases={cases ?? []}
                onApply={(input) => {
                  mutation.mutate({ input, kind: 'apply' });
                }}
                onReauthenticate={onReauthenticate}
                pending={mutation.isPending}
                recentlyAuthenticated={recentlyAuthenticated}
              />
            )}
            {actions === null || agents === undefined ? null : (
              <AgentModerationSettings
                {...agents}
                actions={actions}
                catalogId={catalogId}
                onReauthenticate={onReauthenticate}
                recentlyAuthenticated={recentlyAuthenticated}
              />
            )}
          </div>
          <div className="moderation-governance__secondary">
            {actions === null ? null : (
              <ModerationActionLedger
                actions={actions}
                agentLedger={agentLedger}
                now={now}
                onReverse={(actionId) => {
                  mutation.mutate({ actionId, kind: 'reverse' });
                }}
                pendingActionId={pendingActionId}
                recentlyAuthenticated={recentlyAuthenticated}
              />
            )}
            {audit === null ? null : <ModerationAuditLedger events={audit} />}
          </div>
        </div>
      )}
    </div>
  );
}

function ModerationBoundary({ onRetry }: { readonly onRetry: () => Promise<void> }) {
  const { t } = useTranslation();
  return (
    <div className="moderation-boundary" role="alert">
      <CircleAlert aria-hidden="true" />
      <strong>{t('moderation.governance.failure', { code: 'moderation.unavailable' })}</strong>
      <Button
        icon={<RefreshCw aria-hidden="true" />}
        onClick={() => void onRetry()}
        size="compact"
        tone="quiet"
      >
        {t('moderation.governance.retry')}
      </Button>
    </div>
  );
}

function ModerationFailureNotice({
  failure,
  onReauthenticate,
  reversingMute,
}: {
  readonly failure: ModerationFailure;
  readonly onReauthenticate: () => void;
  /** 这次没做成的是撤回一条禁言。 */
  readonly reversingMute: boolean;
}) {
  const { t } = useTranslation();
  const reauthenticate = failure.code === 'authentication.reauthentication_required';
  // 撤回禁言回冲突：这个人的禁言别处正在处理（后台在到期解除、另一个管理员在撤），或者有一条正在落。
  const busy = reversingMute && failure.code === 'moderation.conflict';
  // 对象是这间房的管理员：禁言、踢出、封禁他，和撤回这些动作，都只有房主能做。
  const ownerOnly = failure.code === 'private_room.owner_only';
  return (
    <div className="moderation-inline-failure" role="alert">
      {failure.retryable ? <LoaderCircle aria-hidden="true" /> : <CircleAlert aria-hidden="true" />}
      <span>
        {busy
          ? t('moderation.governance.reverseBusy')
          : ownerOnly
            ? t('moderation.governance.ownerOnly')
            : t('moderation.governance.failure', { code: failure.code })}
      </span>
      {reauthenticate ? (
        <Button onClick={onReauthenticate} size="compact" tone="quiet">
          {t('moderation.governance.action.reauthenticate')}
        </Button>
      ) : null}
    </div>
  );
}

/**
 * 台账按哪个时刻算过没过期限。到下一个到期的那一刻重画一次：台账改说“正在解除”，读台账的查询也跟着
 * 隔一会儿再读，等服务器解除。
 */
function useLedgerClock(actions: readonly ModerationAction[] | null): number {
  const [now, setNow] = useState(() => Date.now());
  const next = actions === null ? null : nextModerationExpiry(actions, now);
  useEffect(() => {
    if (next === null) {
      return undefined;
    }
    const timer = setTimeout(
      () => {
        setNow(Date.now());
      },
      Math.min(Math.max(next - Date.now(), 0), MAXIMUM_TIMER_MS),
    );
    return () => {
      clearTimeout(timer);
    };
  }, [next, now]);
  return now;
}
