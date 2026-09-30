import { Button } from '@agent-room/ui-system';
import { useMutation, useQueryClient } from '@tanstack/react-query';
import { CircleAlert, LoaderCircle, RefreshCw } from 'lucide-react';
import { useMemo, useState } from 'react';
import { useTranslation } from 'react-i18next';

import {
  automationGrantListQueryKey,
  useAutomationGrantList,
} from '@/features/automation/data/automation-grant-queries';
import type {
  AutomationGrantFailure,
  AutomationGrantGateway,
  CreateAutomationGrantInput,
} from '@/features/automation/domain/automation-grant';
import { AutomationGrantForm } from '@/features/automation/ui/automation-grant-form';
import { AutomationGrantList } from '@/features/automation/ui/automation-grant-list';
import { useAgentInstances } from '@/features/security/data/access-management-queries';
import type { AccessManagementGateway } from '@/features/security/domain/access-management';
import { BrowserUuidV7Factory } from '@/shared/ids/browser-uuid-v7-factory';
import {
  clearAutomationGrantDraft,
  readAutomationGrantDraft,
  saveAutomationGrantDraft,
} from '@/features/automation/adapters/automation-grant-draft';

export type AutomationSettingsProps = {
  readonly accessManagement: AccessManagementGateway;
  readonly automation: AutomationGrantGateway;
  readonly catalogId: string;
  readonly principalId: string;
  readonly roomName: string;
};

type AutomationCommand =
  | { readonly input: CreateAutomationGrantInput; readonly kind: 'create' }
  | { readonly grantId: string; readonly kind: 'revoke' };

/** 这间房里有没有没提交完的自动发言授权（重新登录回来时要接着填）。 */
export function hasAutomationDraftFor(principalId: string, catalogId: string): boolean {
  const draft = readAutomationGrantDraft(principalId);
  return draft.ok && draft.value?.input.roomCatalogId === catalogId;
}

/**
 * 房间设置里的“自动发言”：给一个 Agent 限定范围的自动发言授权，以及这间房现有的授权。
 * 提交前先把草稿存在本机，重新登录回来能接着填。
 */
export function AutomationSettings({
  accessManagement,
  automation,
  catalogId,
  principalId,
  roomName,
}: AutomationSettingsProps) {
  const { t } = useTranslation();
  const queryClient = useQueryClient();
  const grantsQuery = useAutomationGrantList(automation);
  const instancesQuery = useAgentInstances(accessManagement);
  const identifiers = useMemo(() => new BrowserUuidV7Factory(), []);
  const [savedDraft, setSavedDraft] = useState(() => readAutomationGrantDraft(principalId));
  const initialDraft =
    savedDraft.ok && savedDraft.value?.input.roomCatalogId === catalogId
      ? savedDraft.value.input
      : undefined;
  const [draftFailure, setDraftFailure] = useState(!savedDraft.ok);
  const grants = grantsQuery.data?.ok === true ? grantsQuery.data.value : [];
  const roomGrants = grants.filter((grant) => grant.roomCatalogId === catalogId);
  const instances =
    instancesQuery.data?.ok === true
      ? instancesQuery.data.value.filter((instance) => instance.status !== 'revoked')
      : [];
  const mutation = useMutation({
    mutationFn: async (command: AutomationCommand) =>
      command.kind === 'create'
        ? await automation.create(identifiers.next(), command.input)
        : await automation.revoke(command.grantId),
    onSuccess: async (result, command) => {
      if (result.ok) {
        if (command.kind === 'create') {
          setDraftFailure(!clearAutomationGrantDraft(principalId, catalogId).ok);
          setSavedDraft({ ok: true, value: null });
        }
        await queryClient.invalidateQueries({ queryKey: automationGrantListQueryKey });
      }
    },
  });
  const failure = mutation.data?.ok === false ? mutation.data.error : null;
  const listFailure = grantsQuery.data?.ok === false ? grantsQuery.data.error : null;
  const instancesFailure = instancesQuery.data?.ok === false ? instancesQuery.data.error : null;
  const pendingGrantId =
    mutation.isPending && mutation.variables.kind === 'revoke' ? mutation.variables.grantId : null;
  const successKind = mutation.data?.ok === true ? (mutation.variables?.kind ?? null) : null;

  const retry = async (): Promise<void> => {
    await Promise.all([grantsQuery.refetch(), instancesQuery.refetch()]);
  };

  return (
    <div className="automation-settings">
      <p aria-atomic="true" aria-live="polite" className="sr-only">
        {successKind === null
          ? ''
          : t(
              successKind === 'create'
                ? 'automation.success.created'
                : 'automation.success.revoked',
            )}
      </p>
      {grantsQuery.isPending || instancesQuery.isPending ? (
        <div className="automation-boundary" role="status">
          <LoaderCircle aria-hidden="true" className="automation-spin" />
          <strong>{t('automation.loading')}</strong>
        </div>
      ) : listFailure !== null || instancesFailure !== null ? (
        <div className="automation-boundary" role="alert">
          <CircleAlert aria-hidden="true" />
          <div>
            <strong>{t('automation.loadFailed')}</strong>
            <code>{(listFailure ?? instancesFailure)?.code}</code>
          </div>
          <Button
            icon={<RefreshCw aria-hidden="true" />}
            onClick={() => void retry()}
            size="compact"
            tone="quiet"
          >
            {t('automation.retry')}
          </Button>
        </div>
      ) : (
        <>
          {draftFailure ? (
            <p className="automation-inline-failure" role="alert">
              {t('automation.draft.failed')}
            </p>
          ) : null}
          {failure === null ? null : <AutomationFailure failure={failure} />}
          <AutomationGrantForm
            catalogId={catalogId}
            instances={instances}
            {...(initialDraft === undefined ? {} : { initialDraft })}
            onCreate={(input) => {
              setDraftFailure(!saveAutomationGrantDraft(principalId, input).ok);
              mutation.mutate({ input, kind: 'create' });
            }}
            pending={mutation.isPending}
            roomName={roomName}
          />
          <AutomationGrantList
            grants={roomGrants}
            instances={instances}
            onRevoke={(grantId) => {
              mutation.mutate({ grantId, kind: 'revoke' });
            }}
            pendingGrantId={pendingGrantId}
          />
        </>
      )}
    </div>
  );
}

function AutomationFailure({ failure }: { readonly failure: AutomationGrantFailure }) {
  const { t } = useTranslation();
  return (
    <p className="automation-inline-failure" role="alert">
      <CircleAlert aria-hidden="true" />
      <span>
        {t('automation.failure', { code: failure.code })}
        {failure.correlationId === undefined ? null : ` · ${failure.correlationId}`}
      </span>
    </p>
  );
}
