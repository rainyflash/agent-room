import type {
  AutomationGrant,
  AutomationGrantGateway,
} from '@/features/automation/domain/automation-grant';
import type {
  DesktopRuntimeFailure,
  DesktopRuntimeGateway,
  HostSessionDiagnostics,
} from '../domain/desktop-runtime';
import { receptionGrants } from '../domain/reception';
import { err, ok, type Result } from '@/shared/result';

export const RECEPTION_AUTHORIZATION_DAYS = 7;
// Replies are visible to every member of the selected room, including later arrivals.
// The room, Agent instance, sender policy and reply-only limit remain fixed.
const RECEPTION_AUDIENCE = 'any_room_member';

/** A user's single explicit action authorizes room replies, registers the binding, then starts it.
 * Partial failures stay visible; a configured but stopped task can be resumed from its card.
 */
export async function enableReception(
  input: {
    readonly session: HostSessionDiagnostics;
    readonly principalId: string;
    readonly grantId: string;
    readonly grants: readonly AutomationGrant[];
    readonly now: number;
  },
  dependencies: {
    readonly automation: Pick<AutomationGrantGateway, 'create'>;
    readonly runtime: Pick<DesktopRuntimeGateway, 'configureReceiver' | 'receiverAction'>;
  },
): Promise<Result<void, DesktopRuntimeFailure>> {
  const { session } = input;
  const offer = session.receptionOffer;
  const agentId = session.session.agentId;
  const { runtime, automation } = dependencies;
  if (offer?.roomCatalogId == null || agentId === null || session.session.state !== 'ready') {
    return err({ code: 'receiver.session_not_ready', retryable: true });
  }
  if (runtime.configureReceiver === undefined || runtime.receiverAction === undefined) {
    return err({ code: 'receiver.unavailable', retryable: false });
  }
  let grant = receptionGrants(
    input.grants,
    agentId,
    offer.roomCatalogId,
    offer.instanceId,
    input.now,
  ).find((candidate) => candidate.audience === RECEPTION_AUDIENCE);
  if (grant === undefined) {
    const created = await automation.create(input.grantId, {
      agentId,
      agentInstanceId: offer.instanceId,
      roomCatalogId: offer.roomCatalogId,
      audience: RECEPTION_AUDIENCE,
      messageKinds: ['reply'],
      impactAcknowledged: true,
      lifetimeSeconds: RECEPTION_AUTHORIZATION_DAYS * 24 * 60 * 60,
      maxMessagesPerMinute: 10,
      maxTotalMessages: 1000,
      requiresRiskScan: true,
    });
    if (!created.ok) return created;
    grant = created.value;
    if (
      grant.audience !== RECEPTION_AUDIENCE ||
      receptionGrants([grant], agentId, offer.roomCatalogId, offer.instanceId, grant.startsAtUnixMs)
        .length !== 1
    ) {
      return err({ code: 'receiver.authorization_mismatch', retryable: false });
    }
  }
  const configured = await runtime.configureReceiver({
    sessionId: session.session.sessionId,
    principalId: input.principalId,
    automationGrantId: grant.grantId,
    executable: null,
  });
  if (!configured.ok) return configured;
  const started = await runtime.receiverAction(offer.task.taskId, { action: 'start' });
  return started.ok ? ok(undefined) : started;
}
