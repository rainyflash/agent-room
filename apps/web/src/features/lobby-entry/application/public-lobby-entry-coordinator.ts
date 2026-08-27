import type {
  PublicLobbyEntryFailure,
  PublicLobbyEntryGateway,
  PublicLobbyEntryTarget,
  PublicLobbyRouteTarget,
  PublicLobbyMatrixGateway,
} from '@/features/lobby-entry/domain/public-lobby-entry';
import { publicLobbyRouteTargetSchema } from '@/features/lobby-entry/domain/public-lobby-entry';
import { err } from '@/shared/result';
import type { Result } from '@/shared/result';

export class PublicLobbyEntryCoordinator {
  constructor(
    private readonly entry: PublicLobbyEntryGateway,
    private readonly matrix: PublicLobbyMatrixGateway,
  ) {}

  async enter(catalogId: string): Promise<Result<PublicLobbyEntryTarget, PublicLobbyEntryFailure>> {
    const target = await this.entry.resolve(catalogId);
    if (!target.ok) return target;
    const joined = await this.matrix.join(target.value.matrixRoomId);
    return joined.ok ? target : joined;
  }

  async enterKnown(
    target: PublicLobbyRouteTarget,
  ): Promise<Result<PublicLobbyRouteTarget, PublicLobbyEntryFailure>> {
    const parsed = publicLobbyRouteTargetSchema.safeParse(target);
    if (!parsed.success) {
      return err({ code: 'lobby_entry.known_target_invalid', retryable: false });
    }
    const joined = await this.matrix.join(parsed.data.matrixRoomId);
    return joined.ok ? { ok: true, value: parsed.data } : joined;
  }
}
