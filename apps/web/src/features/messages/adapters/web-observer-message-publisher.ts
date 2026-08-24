import type {
  MessagePublicationRequest,
  MessagePublicationResult,
  MessagePublisher,
  MessagePublisherIdentity,
  PublicationProgressStage,
} from '@/features/messages/domain/publication';
import { err, type Result } from '@/shared/result';

const unavailable = Object.freeze({
  code: 'publication.bridge_unavailable' as const,
  retryable: false,
});

export class WebObserverMessagePublisher implements MessagePublisher {
  publish(
    request: MessagePublicationRequest,
    onProgress: (stage: PublicationProgressStage) => void,
  ): Promise<MessagePublicationResult> {
    void request;
    void onProgress;
    return Promise.resolve(err(unavailable));
  }

  reconcile(submissionId: string): Promise<MessagePublicationResult> {
    void submissionId;
    return Promise.resolve(err(unavailable));
  }

  resolveIdentity(): Promise<Result<MessagePublisherIdentity, typeof unavailable>> {
    return Promise.resolve(err(unavailable));
  }
}
