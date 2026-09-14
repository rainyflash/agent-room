import type { ReceiverView } from './reception';
import type { HostSessionDiagnostics } from './desktop-runtime';
import type { AutomationGrant } from '@/features/automation/domain/automation-grant';

/** A configured worker has no agent ID until it starts; retain its controls if startup fails. */
export function receiverAgentId(
  view: ReceiverView,
  sessions: readonly HostSessionDiagnostics[],
  grants: readonly Pick<AutomationGrant, 'agentId' | 'grantId'>[],
): string | null {
  return (
    view.state.agentId ??
    sessions.find((session) => session.sessionKey === view.state.binding.session.sessionKey)
      ?.session.agentId ??
    grants.find((grant) => grant.grantId === view.state.binding.automationGrantId)?.agentId ??
    null
  );
}

export function receiverStatus(view: ReceiverView) {
  const record =
    view.progress?.type === 'delivery' ? view.progress.record : view.state.lastDelivery;
  if (view.progress?.type === 'reconnecting') return 'reconnecting';
  if (view.failure || record?.failure || record?.stage === 'needs_review') return 'needs_review';
  if (view.running && (record?.stage === 'running' || record?.stage === 'verifying'))
    return record.stage;
  if (view.state.checkpoint.state === 'pending') return 'needs_review';
  // The worker must have reported readiness before we promise that it is listening.
  if (view.running) return view.progress === null ? 'starting' : 'waiting';
  return 'paused';
}
