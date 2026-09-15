import type { MatrixClientSource } from '@/shared/matrix/matrix-client-registry';
import type { InboxMatrixSource } from '../domain/inbox';

export class MatrixInboxSource implements InboxMatrixSource {
  constructor(private readonly clients: MatrixClientSource) {}
  readonly accountId = () => this.clients.current()?.getUserId() ?? null;
  readonly isJoined = (roomId: string) =>
    this.clients.current()?.getRoom(roomId)?.getMyMembership() === 'join';
  readonly isIgnored = (userId: string) =>
    this.clients.current()?.getIgnoredUsers().includes(userId) ?? false;
  readonly subscribe = (listener: () => void) => this.clients.subscribe(listener);
}
