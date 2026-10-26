import type {
  BlockedAgent,
  DirectBlockRegistry,
  DirectBlockTarget,
  DirectContact,
  DirectSession,
  DirectSessionFailure,
  DirectSessionGateway,
  DirectSessionMatrixGateway,
} from '@/features/direct-sessions/domain/direct-session';
import { ok, type Result } from '@/shared/result';

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

  /**
   * 你屏蔽着它没有：这台设备先记下的，或者聊天账户的忽略名单里有它（在别的设备上屏蔽的也算）。
   * 房间里看不看得到它说的话，看的是忽略名单。
   */
  isBlocked(target: DirectBlockTarget): boolean {
    return (
      this.#localBlocks.has(target.agentId) ||
      this.#matrix.ignoredUsers().some((user) => user.matrixUserId === target.matrixUserId)
    );
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
    target: DirectBlockTarget,
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

  /**
   * 设置里列出的屏蔽：服务器记着的在前，再是只在聊天账户忽略名单里的（以前服务器没记上的那些）。
   */
  async listBlocked(): Promise<Result<readonly BlockedAgent[], DirectSessionFailure>> {
    const listed = await this.#controlPlane.listBlocks();
    if (!listed.ok) {
      return listed;
    }
    const recorded = listed.value.map((block): BlockedAgent =>
      Object.freeze({
        agentId: block.target.agentId,
        blockedAtUnixMs: block.blockedAtUnixMs,
        displayName: block.target.displayName,
        matrixUserId: block.target.matrixUserId,
      }),
    );
    const known = new Set(recorded.map((block) => block.matrixUserId));
    const ignoredOnly = this.#matrix
      .ignoredUsers()
      .filter((user) => !known.has(user.matrixUserId))
      .map((user): BlockedAgent =>
        Object.freeze({
          agentId: null,
          blockedAtUnixMs: null,
          displayName: user.displayName,
          matrixUserId: user.matrixUserId,
        }),
      );
    return ok(Object.freeze([...recorded, ...ignoredOnly]));
  }

  /** 解除设置里列出的一个屏蔽；只在忽略名单里的，从忽略名单里拿掉就好。 */
  async unblock(blocked: BlockedAgent): Promise<Result<void, DirectSessionFailure>> {
    if (blocked.agentId === null) {
      return await this.#matrix.setIgnored(blocked.matrixUserId, false);
    }
    const result = await this.setBlocked(
      { agentId: blocked.agentId, matrixUserId: blocked.matrixUserId },
      false,
    );
    return result.ok ? ok(undefined) : result;
  }

  markDisplayed(
    roomId: string,
    matrixEventId: string,
  ): Promise<Result<void, DirectSessionFailure>> {
    return this.#matrix.markDisplayed(roomId, matrixEventId);
  }
}
