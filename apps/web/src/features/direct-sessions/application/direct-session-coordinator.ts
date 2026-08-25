import type {
  DirectAgent,
  DirectContact,
  DirectSession,
  DirectSessionFailure,
  DirectSessionGateway,
  DirectSessionMatrixGateway,
} from '@/features/direct-sessions/domain/direct-session';
import type { Result } from '@/shared/result';

export class DirectSessionCoordinator {
  readonly #controlPlane: DirectSessionGateway;
  readonly #matrix: DirectSessionMatrixGateway;

  constructor(controlPlane: DirectSessionGateway, matrix: DirectSessionMatrixGateway) {
    this.#controlPlane = controlPlane;
    this.#matrix = matrix;
  }

  async open(targetAgentId: string): Promise<Result<DirectSession, DirectSessionFailure>> {
    const opened = await this.#controlPlane.open(targetAgentId);
    if (!opened.ok) {
      return opened;
    }
    const prepared = await this.#matrix.prepare(opened.value);
    return prepared.ok ? opened : prepared;
  }

  async setBlocked(
    target: DirectAgent,
    blocked: boolean,
  ): Promise<Result<DirectContact, DirectSessionFailure>> {
    if (blocked) {
      const persisted = await this.#controlPlane.setBlocked(target.agentId, true);
      if (!persisted.ok) {
        return persisted;
      }
      const ignored = await this.#matrix.setIgnored(target.matrixUserId, true);
      return ignored.ok ? persisted : ignored;
    }

    const unignored = await this.#matrix.setIgnored(target.matrixUserId, false);
    if (!unignored.ok) {
      return unignored;
    }
    return await this.#controlPlane.setBlocked(target.agentId, false);
  }

  markDisplayed(
    roomId: string,
    matrixEventId: string,
  ): Promise<Result<void, DirectSessionFailure>> {
    return this.#matrix.markDisplayed(roomId, matrixEventId);
  }
}
