import { useQuery } from '@tanstack/react-query';
import { useState } from 'react';
import { useTranslation } from 'react-i18next';
import { useAppServices } from '@/app/app-services';
import { agentRosterPolicySchema } from '../domain/agent-roster-policy';

export function AgentRosterPolicyPanel({
  catalogId,
  days,
}: {
  readonly catalogId: string;
  readonly days: number;
}) {
  const { t } = useTranslation();
  const { moderation, agentRosterPolicy } = useAppServices();
  const [selected, setSelected] = useState(days);
  const [state, setState] = useState<'idle' | 'saving' | 'saved' | 'failed'>('idle');
  const permission = useQuery({
    queryKey: ['agent-roster-permission', catalogId],
    queryFn: () => moderation.inspectCapabilities(catalogId),
  });
  const canManage = permission.data?.ok === true && permission.data.value.canModerateRoom;
  const save = async () => {
    const policy = agentRosterPolicySchema.safeParse({
      schemaVersion: 1,
      archiveAfterDays: selected,
    });
    if (!policy.success || !agentRosterPolicy) return;
    setState('saving');
    const result = await agentRosterPolicy.update(catalogId, policy.data);
    setState(result.ok ? 'saved' : 'failed');
  };
  return (
    <details className="roster-policy">
      <summary>{t('agentState.policy')}</summary>
      <p>{t('agentState.policyHint', { days })}</p>
      {canManage && agentRosterPolicy ? (
        <div>
          <label>
            {t('agentState.policyDays')}
            <select
              value={selected}
              disabled={state === 'saving'}
              onChange={(event) => {
                setSelected(Number(event.target.value));
                setState('idle');
              }}
            >
              {[1, 7, 30].map((count) => (
                <option key={count} value={count}>
                  {t('agentState.days', { count })}
                </option>
              ))}
            </select>
          </label>
          <button
            type="button"
            disabled={state === 'saving'}
            onClick={() => {
              void save();
            }}
          >
            {t(state === 'saving' ? 'agentState.saving' : 'agentState.save')}
          </button>
        </div>
      ) : null}
      {state === 'failed' ? (
        <p role="alert">{t('agentState.saveFailed')}</p>
      ) : state === 'saved' ? (
        <p role="status">{t('agentState.saved')}</p>
      ) : null}
    </details>
  );
}
