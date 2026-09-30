import { Button } from '@agent-room/ui-system';
import { CircleAlert, LoaderCircle, RefreshCw } from 'lucide-react';
import { useMutation, useQueryClient } from '@tanstack/react-query';
import { useMemo } from 'react';
import { useTranslation } from 'react-i18next';

import {
  moderationActionListQueryKey,
  moderationAuditListQueryKey,
  moderationRoomCaseListQueryKey,
  useModerationActions,
  useModerationAudit,
  useModerationCapabilities,
  useModerationRoomCases,
} from '@/features/moderation/data/moderation-queries';
import type {
  ApplyModerationActionInput,
  ModerationFailure,
  ModerationGateway,
} from '@/features/moderation/domain/moderation';
import { ModerationActionForm } from '@/features/moderation/ui/moderation-action-form';
import {
  ModerationActionLedger,
  ModerationAuditLedger,
  ModerationCaseLedger,
} from '@/features/moderation/ui/moderation-ledger';
import { BrowserUuidV7Factory } from '@/shared/ids/browser-uuid-v7-factory';

export type ModerationSettingsProps = {
  readonly catalogId: string;
  readonly gateway: ModerationGateway;
  readonly onReauthenticate: () => void;
  readonly recentlyAuthenticated: boolean;
};

type ModerationCommand =
  | { readonly input: ApplyModerationActionInput; readonly kind: 'apply' }
  | { readonly actionId: string; readonly kind: 'reverse' };

/**
 * 房间设置里的“治理”：举报、处理动作和审计记录。只有房间管理者（或能看审计的人）看得到这一节，
 * 没有权限时不去读任何受限资源。
 */
export function ModerationSettings({
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
  const cases = casesQuery.data?.ok === true ? casesQuery.data.value : null;
  const actions = actionsQuery.data?.ok === true ? actionsQuery.data.value : null;
  const audit = auditQuery.data?.ok === true ? auditQuery.data.value : null;
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
        <ModerationFailureNotice failure={failure} onReauthenticate={onReauthenticate} />
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
          </div>
          <div className="moderation-governance__secondary">
            {actions === null ? null : (
              <ModerationActionLedger
                actions={actions}
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
}: {
  readonly failure: ModerationFailure;
  readonly onReauthenticate: () => void;
}) {
  const { t } = useTranslation();
  const reauthenticate = failure.code === 'authentication.reauthentication_required';
  return (
    <div className="moderation-inline-failure" role="alert">
      {failure.retryable ? <LoaderCircle aria-hidden="true" /> : <CircleAlert aria-hidden="true" />}
      <span>{t('moderation.governance.failure', { code: failure.code })}</span>
      {reauthenticate ? (
        <Button onClick={onReauthenticate} size="compact" tone="quiet">
          {t('moderation.governance.action.reauthenticate')}
        </Button>
      ) : null}
    </div>
  );
}
