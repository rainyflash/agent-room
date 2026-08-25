import type {
  CreatePrivateRoomInput,
  PrivateRoom,
  PrivateRoomFailure,
  PrivateRoomGateway,
  PrivateRoomMatrixGateway,
} from '@/features/private-rooms/domain/private-room';
import { ok, type Result } from '@/shared/result';

export class PrivateRoomCoordinator {
  readonly #matrix: PrivateRoomMatrixGateway;
  readonly #rooms: PrivateRoomGateway;

  constructor(rooms: PrivateRoomGateway, matrix: PrivateRoomMatrixGateway) {
    this.#rooms = rooms;
    this.#matrix = matrix;
  }

  async createAndJoin(
    catalogId: string,
    input: CreatePrivateRoomInput,
  ): Promise<Result<PrivateRoom, PrivateRoomFailure>> {
    const created = await this.#rooms.create(catalogId, input);
    if (!created.ok) {
      return created;
    }
    const joined = await this.#matrix.join(created.value.matrixRoomId);
    return joined.ok ? ok(created.value) : joined;
  }

  async open(room: PrivateRoom): Promise<Result<PrivateRoom, PrivateRoomFailure>> {
    const joined = await this.#matrix.join(room.matrixRoomId);
    return joined.ok ? ok(room) : joined;
  }

  async accept(room: PrivateRoom): Promise<Result<PrivateRoom, PrivateRoomFailure>> {
    const joined = await this.#matrix.join(room.matrixRoomId);
    return joined.ok ? await this.#rooms.accept(room.catalogId) : joined;
  }

  async decline(room: PrivateRoom): Promise<Result<PrivateRoom, PrivateRoomFailure>> {
    const left = await this.#matrix.leave(room.matrixRoomId);
    return left.ok ? await this.#rooms.decline(room.catalogId) : left;
  }

  async leave(room: PrivateRoom): Promise<Result<PrivateRoom, PrivateRoomFailure>> {
    const left = await this.#matrix.leave(room.matrixRoomId);
    return left.ok ? await this.#rooms.leave(room.catalogId) : left;
  }
}
