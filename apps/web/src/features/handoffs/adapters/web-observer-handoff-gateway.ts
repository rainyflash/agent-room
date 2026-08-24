import type {
  HandoffApprovalRequest,
  HandoffFailure,
  HandoffGateway,
  HandoffSnapshot,
  HandoffSubmissionOutcome,
  HandoffTarget,
} from '@/features/handoffs/domain/handoff';
import { err, type Result } from '@/shared/result';

const unavailable: HandoffFailure = Object.freeze({
  code: 'handoff.bridge_unavailable',
  retryable: false,
});

export class WebObserverHandoffGateway implements HandoffGateway {
  approve(
    request: HandoffApprovalRequest,
  ): Promise<Result<HandoffSubmissionOutcome, HandoffFailure>> {
    void request;
    return Promise.resolve(err(unavailable));
  }

  listTargets(roomId: string): Promise<Result<readonly HandoffTarget[], HandoffFailure>> {
    void roomId;
    return Promise.resolve(err(unavailable));
  }

  reconcile(handoffId: string): Promise<Result<HandoffSnapshot, HandoffFailure>> {
    void handoffId;
    return Promise.resolve(err(unavailable));
  }

  revoke(handoffId: string): Promise<Result<HandoffSnapshot, HandoffFailure>> {
    void handoffId;
    return Promise.resolve(err(unavailable));
  }
}
