import { typingTimeoutMs, type TypingNotifier } from '../domain/typing';
import type { MatrixClientSource } from '@/shared/matrix/matrix-client-registry';

/** 用 Matrix 的“正在输入”告诉房间里的人。发不出去不要紧，不打扰输入。 */
export class MatrixTypingNotifier implements TypingNotifier {
  readonly #clients: MatrixClientSource;

  constructor(clients: MatrixClientSource) {
    this.#clients = clients;
  }

  typing(roomId: string, typing: boolean): void {
    const client = this.#clients.current();
    if (client?.getRoom(roomId)?.getMyMembership() !== 'join') return;
    client.sendTyping(roomId, typing, typingTimeoutMs).catch(() => undefined);
  }
}
