import { Button, Details } from '@agent-room/ui-system';
import { useMutation, useQueryClient } from '@tanstack/react-query';
import { Bot, CircleAlert, RotateCcw, X } from 'lucide-react';
import { useState } from 'react';
import { useTranslation } from 'react-i18next';

import {
  agentModerationLedgerQueryKey,
  moderationActionListQueryKey,
  moderationAuditListQueryKey,
  roomModerationGrantListQueryKey,
  useAgentModerationLedger,
  useRoomModerationGrants,
} from '@/features/moderation/data/moderation-queries';
import {
  grantsInEffect,
  type AgentModerationGateway,
  type GrantInEffect,
  type GrantReversal,
  type GrantRoomModerationInput,
  type RoomModerationGrant,
} from '@/features/moderation/domain/agent-moderation';
import type { ModerationAction, ModerationFailure } from '@/features/moderation/domain/moderation';
import {
  AgentModerationGrantForm,
  type GrantableAgent,
} from '@/features/moderation/ui/agent-moderation-grant-form';
import { useAgentInstances } from '@/features/security/data/access-management-queries';
import type { AccessManagementGateway } from '@/features/security/domain/access-management';
import { ok, type Result } from '@/shared/result';

export type AgentModerationSettingsProps = {
  readonly accessManagement: AccessManagementGateway;
  /** 台账里的动作，算每份授权下还生效着几条。 */
  readonly actions: readonly ModerationAction[];
  readonly catalogId: string;
  readonly gateway: AgentModerationGateway;
  /** 给授权的人叫什么：按成员名单换成昵称，认不出来就不写。 */
  readonly nameOf: (principalId: string) => string | undefined;
  readonly onReauthenticate: () => void;
  readonly principalId: string;
  readonly recentlyAuthenticated: boolean;
  readonly roomName: string;
};

type GrantCommand =
  | { readonly input: GrantRoomModerationInput; readonly kind: 'grant' }
  | { readonly grantId: string; readonly kind: 'revoke' }
  | { readonly grantId: string; readonly kind: 'reverse' };

type GrantOutcome = Result<GrantReversal | null, ModerationFailure>;

/**
 * 治理里的“交给 Agent 管”（`specs/public-rooms/design.md`“房主的 Agent 管房间”）：房主和管理员把房间交给自己的
 * Agent 管、列出在用的授权和撤掉；授权撤了或到期以后它之前做的照旧生效，这里能把一份授权下还生效着的一次全撤。
 */
export function AgentModerationSettings({
  accessManagement,
  actions,
  catalogId,
  gateway,
  nameOf,
  onReauthenticate,
  principalId,
  recentlyAuthenticated,
  roomName,
}: AgentModerationSettingsProps) {
  const { t } = useTranslation();
  const queryClient = useQueryClient();
  const grantsQuery = useRoomModerationGrants(gateway, catalogId);
  const ledgerQuery = useAgentModerationLedger(gateway, catalogId);
  const instancesQuery = useAgentInstances(accessManagement);
  const grants = grantsQuery.data?.ok === true ? grantsQuery.data.value : null;
  const inEffect =
    ledgerQuery.data?.ok === true ? grantsInEffect(actions, ledgerQuery.data.value) : [];
  // 读到自己的 Agent 之前不出表单，免得先说一句“还没有能交给它管的 Agent”。
  const agents =
    instancesQuery.data?.ok === true ? grantableAgents(instancesQuery.data.value) : null;
  const instancesFailure = instancesQuery.data?.ok === false ? instancesQuery.data.error : null;
  const mutation = useMutation({
    mutationFn: async (command: GrantCommand): Promise<GrantOutcome> => {
      switch (command.kind) {
        case 'grant': {
          const granted = await gateway.grant(catalogId, command.input);
          return granted.ok ? ok(null) : granted;
        }
        case 'revoke': {
          const revoked = await gateway.revokeGrant(catalogId, command.grantId);
          return revoked.ok ? ok(null) : revoked;
        }
        case 'reverse':
          return await gateway.reverseGrant(catalogId, command.grantId);
      }
    },
    onSuccess: async (_result, command) => {
      await Promise.all(
        (command.kind === 'reverse'
          ? [
              moderationActionListQueryKey(catalogId),
              moderationAuditListQueryKey(catalogId),
              agentModerationLedgerQueryKey(catalogId),
            ]
          : [roomModerationGrantListQueryKey(catalogId)]
        ).map(async (queryKey) => {
          await queryClient.invalidateQueries({ queryKey });
        }),
      );
    },
  });
  const failure = mutation.data?.ok === false ? mutation.data.error : null;
  const reversal =
    mutation.data?.ok === true && mutation.variables?.kind === 'reverse'
      ? mutation.data.value
      : null;
  const pendingGrantId =
    mutation.isPending && mutation.variables.kind !== 'grant' ? mutation.variables.grantId : null;

  return (
    <div className="moderation-agents">
      {failure === null ? null : (
        <div className="moderation-inline-failure" role="alert">
          <CircleAlert aria-hidden="true" />
          <span>{t('moderation.agents.failure', { code: failure.code })}</span>
          {failure.code === 'authentication.reauthentication_required' ? (
            <Button onClick={onReauthenticate} size="compact" tone="quiet">
              {t('moderation.governance.action.reauthenticate')}
            </Button>
          ) : null}
        </div>
      )}
      {instancesFailure === null ? null : (
        <div className="moderation-inline-failure" role="alert">
          <CircleAlert aria-hidden="true" />
          <span>{t('moderation.agents.failure', { code: instancesFailure.code })}</span>
        </div>
      )}
      {agents === null ? null : (
        <AgentModerationGrantForm
          agents={agents}
          onGrant={(input) => {
            mutation.mutate({ input, kind: 'grant' });
          }}
          onReauthenticate={onReauthenticate}
          pending={mutation.isPending && mutation.variables.kind === 'grant'}
          recentlyAuthenticated={recentlyAuthenticated}
          roomName={roomName}
        />
      )}
      {grants === null ? null : (
        <GrantList
          grants={grants}
          nameOf={(grantedBy) =>
            grantedBy === principalId ? t('moderation.agents.grantedByYou') : nameOf(grantedBy)
          }
          onRevoke={(grantId) => {
            mutation.mutate({ grantId, kind: 'revoke' });
          }}
          pendingGrantId={pendingGrantId}
        />
      )}
      {inEffect.length === 0 && reversal === null ? null : (
        <GrantsInEffect
          inEffect={inEffect}
          onReauthenticate={onReauthenticate}
          onReverse={(grantId) => {
            mutation.mutate({ grantId, kind: 'reverse' });
          }}
          pendingGrantId={pendingGrantId}
          recentlyAuthenticated={recentlyAuthenticated}
          reversal={reversal}
        />
      )}
    </div>
  );
}

/** 还在用的 Agent（撤掉了的连接不算），同一个 Agent 只列一次。 */
function grantableAgents(
  instances: readonly {
    readonly agentDisplayName: string;
    readonly agentId: string;
    readonly status: string;
  }[],
): readonly GrantableAgent[] {
  const agents = new Map<string, GrantableAgent>();
  for (const instance of instances) {
    if (instance.status === 'revoked' || agents.has(instance.agentId)) continue;
    agents.set(instance.agentId, {
      agentId: instance.agentId,
      displayName: instance.agentDisplayName,
    });
  }
  return [...agents.values()];
}

function GrantList({
  grants,
  nameOf,
  onRevoke,
  pendingGrantId,
}: {
  readonly grants: readonly RoomModerationGrant[];
  readonly nameOf: (principalId: string) => string | undefined;
  readonly onRevoke: (grantId: string) => void;
  readonly pendingGrantId: string | null;
}) {
  const { i18n, t } = useTranslation();
  const formatter = new Intl.DateTimeFormat(i18n.resolvedLanguage, {
    dateStyle: 'medium',
    timeStyle: 'short',
  });
  return (
    <section aria-labelledby="moderation-grants-title" className="moderation-ledger">
      <header>
        <Bot aria-hidden="true" />
        <h3 id="moderation-grants-title">{t('moderation.agents.grants')}</h3>
        <span>{grants.length}</span>
      </header>
      {grants.length === 0 ? (
        <p className="moderation-ledger__empty">{t('moderation.agents.grantsEmpty')}</p>
      ) : (
        <ol>
          {grants.map((grant) => {
            const grantedBy = nameOf(grant.grantedBy);
            return (
              <li key={grant.id}>
                <div className="moderation-ledger__row">
                  <div>
                    <strong>{grant.agentName}</strong>
                    <span>
                      {grant.actions
                        .map((power) => t(`moderation.agents.power.${power}`))
                        .join(t('moderation.agents.separator'))}
                    </span>
                    <span>
                      {t('moderation.agents.until', {
                        time: formatter.format(grant.expiresAtUnixMs),
                      })}
                      {grantedBy === undefined
                        ? ''
                        : ` · ${t('moderation.agents.grantedBy', { name: grantedBy })}`}
                    </span>
                  </div>
                </div>
                <Details
                  className="moderation-ledger__details"
                  summary={t('moderation.agents.rules')}
                >
                  <p className="moderation-agent-grant__rules">{grant.rules}</p>
                </Details>
                <Button
                  disabled={pendingGrantId !== null}
                  icon={<X aria-hidden="true" />}
                  onClick={() => {
                    onRevoke(grant.id);
                  }}
                  size="compact"
                  tone="quiet"
                >
                  {t(
                    pendingGrantId === grant.id
                      ? 'moderation.agents.revoking'
                      : 'moderation.agents.revoke',
                  )}
                </Button>
              </li>
            );
          })}
        </ol>
      )}
      <p className="moderation-ledger__note">{t('moderation.agents.revokeKeeps')}</p>
    </section>
  );
}

/** 每份授权下 Agent 做的、还生效着的：点一下先问一句，确定了再一次全撤。 */
function GrantsInEffect({
  inEffect,
  onReauthenticate,
  onReverse,
  pendingGrantId,
  recentlyAuthenticated,
  reversal,
}: {
  readonly inEffect: readonly GrantInEffect[];
  readonly onReauthenticate: () => void;
  readonly onReverse: (grantId: string) => void;
  readonly pendingGrantId: string | null;
  readonly recentlyAuthenticated: boolean;
  readonly reversal: GrantReversal | null;
}) {
  const { t } = useTranslation();
  const [confirming, setConfirming] = useState<string | null>(null);
  return (
    <section aria-labelledby="moderation-grants-in-effect-title" className="moderation-ledger">
      <header>
        <RotateCcw aria-hidden="true" />
        <h3 id="moderation-grants-in-effect-title">{t('moderation.agents.inEffect')}</h3>
      </header>
      {reversal === null ? null : (
        <p aria-live="polite" className="moderation-ledger__note" role="status">
          {reversal.failed.length === 0
            ? t('moderation.agents.reversed', { count: reversal.reversed.length })
            : t('moderation.agents.reversedPartly', {
                count: reversal.reversed.length,
                failed: reversal.failed.length,
              })}
        </p>
      )}
      {inEffect.length === 0 || recentlyAuthenticated ? null : (
        <div className="moderation-inline-failure" role="note">
          <CircleAlert aria-hidden="true" />
          <span>{t('moderation.governance.action.recentAuth')}</span>
          <Button onClick={onReauthenticate} size="compact" tone="quiet">
            {t('moderation.governance.action.reauthenticate')}
          </Button>
        </div>
      )}
      {inEffect.length === 0 ? null : (
        <ol>
          {inEffect.map((grant) => (
            <li key={grant.grantId}>
              <div className="moderation-ledger__row">
                <div>
                  <strong>{grant.agentName}</strong>
                  <span>
                    {t('moderation.agents.inEffectCount', { count: grant.actionIds.length })}
                  </span>
                </div>
              </div>
              {confirming === grant.grantId ? (
                <div className="moderation-agent-grant__confirm" role="group">
                  <span>
                    {t('moderation.agents.reverseConfirm', {
                      agent: grant.agentName,
                      count: grant.actionIds.length,
                    })}
                  </span>
                  <Button
                    disabled={pendingGrantId !== null}
                    onClick={() => {
                      setConfirming(null);
                      onReverse(grant.grantId);
                    }}
                    size="compact"
                    tone="primary"
                  >
                    {t('moderation.agents.reverseAll')}
                  </Button>
                  <Button
                    onClick={() => {
                      setConfirming(null);
                    }}
                    size="compact"
                    tone="quiet"
                  >
                    {t('moderation.agents.cancel')}
                  </Button>
                </div>
              ) : (
                <Button
                  disabled={!recentlyAuthenticated || pendingGrantId !== null}
                  icon={<RotateCcw aria-hidden="true" />}
                  onClick={() => {
                    setConfirming(grant.grantId);
                  }}
                  size="compact"
                  tone="quiet"
                >
                  {t(
                    pendingGrantId === grant.grantId
                      ? 'moderation.agents.reversing'
                      : 'moderation.agents.reverseAll',
                  )}
                </Button>
              )}
            </li>
          ))}
        </ol>
      )}
    </section>
  );
}
