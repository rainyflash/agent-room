import type {
  DirectAgent,
  DirectBlockRegistry,
  DirectContact,
  DirectSession,
  DirectSessionFailure,
  DirectSessionGateway,
  DirectSessionMatrixGateway,
} from '@/features/direct-sessions/domain/direct-session';
import type { Result } from '@/shared/result';

export class DirectSessionCoordinator {
  readonly #controlPlane: DirectSessionGateway;
  readonly #localBlocks: DirectBlockRegistry;
  readonly #matrix: DirectSessionMatrixGateway;

  constructor(
    controlPlane: DirectSessionGateway,
    matrix: DirectSessionMatrixGateway,
    localBlocks: DirectBlockRegistry,
  ) {
    this.#controlPlane = controlPlane;
    this.#matrix = matrix;
    this.#localBlocks = localBlocks;
  }

  isLocallyBlocked(agentId: string): boolean {
    return this.#localBlocks.has(agentId);
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
      this.#localBlocks.set(target.agentId, true);
      const [persisted, ignored] = await Promise.all([
        this.#controlPlane.setBlocked(target.agentId, true),
        this.#matrix.setIgnored(target.matrixUserId, true),
      ]);
      if (!persisted.ok) {
        return persisted;
      }
      return ignored.ok ? persisted : ignored;
    }

    const persisted = await this.#controlPlane.setBlocked(target.agentId, false);
    if (!persisted.ok) {
      return persisted;
    }
    const unignored = await this.#matrix.setIgnored(target.matrixUserId, false);
    if (!unignored.ok) {
      return unignored;
    }
    this.#localBlocks.set(target.agentId, false);
    return persisted;
  }

  markDisplayed(
    roomId: string,
    matrixEventId: string,
  ): Promise<Result<void, DirectSessionFailure>> {
    return this.#matrix.markDisplayed(roomId, matrixEventId);
  }
}
