import { z } from 'zod';
import {
  agentRecoverySessionSchema,
  agentRecoveryResultSchema,
  agentRecoveryRequestSchema,
  type AgentRecoveryGateway,
  type AgentRecoveryRequest,
  type AgentRecoveryResult,
  type AgentRecoverySession,
  type AgentRecoveryFailure,
} from '@/features/security/domain/agent-recovery';
import { normalizeCommandFailure } from '@/shared/desktop/command-failure';
import { err, ok, type Result } from '@/shared/result';

export type AgentRecoveryTransport = {
  readonly available: () => boolean;
  readonly invoke: (
    command: 'desktop_agent_recovery_sessions' | 'desktop_agent_recovery',
    args: Record<string, unknown>,
  ) => Promise<unknown>;
};

export class TauriAgentRecoveryGateway implements AgentRecoveryGateway {
  constructor(private readonly transport: AgentRecoveryTransport) {}

  async sessions(): Promise<Result<readonly AgentRecoverySession[], AgentRecoveryFailure>> {
    if (!this.transport.available()) return err(unavailable());
    try {
      const raw = await this.transport.invoke('desktop_agent_recovery_sessions', {});
      const parsed = z.array(agentRecoverySessionSchema).max(16).safeParse(raw);
      return parsed.success ? ok(parsed.data) : err(invalidResponse());
    } catch (error: unknown) {
      return err(normalizeCommandFailure(error, 'desktop.agent_recovery.failed'));
    }
  }

  async execute(
    sessionId: string,
    request: AgentRecoveryRequest,
  ): Promise<Result<AgentRecoveryResult, AgentRecoveryFailure>> {
    if (!this.transport.available()) return err(unavailable());
    if (
      !z.uuid().safeParse(sessionId).success ||
      !agentRecoveryRequestSchema.safeParse(request).success
    )
      return err({ code: 'bridge.security.invalid_request', retryable: false });
    try {
      const raw = await this.transport.invoke('desktop_agent_recovery', { sessionId, request });
      const parsed = agentRecoveryResultSchema.safeParse(raw);
      if (!parsed.success || (request.action === 'enable') !== (parsed.data.recoveryKey !== null))
        return err(invalidResponse());
      return ok(parsed.data);
    } catch (error: unknown) {
      return err(normalizeCommandFailure(error, 'desktop.agent_recovery.failed'));
    }
  }
}

function unavailable(): AgentRecoveryFailure {
  return { code: 'desktop.runtime.unavailable', retryable: false };
}
function invalidResponse(): AgentRecoveryFailure {
  return { code: 'desktop.agent_recovery.invalid_response', retryable: false };
}
