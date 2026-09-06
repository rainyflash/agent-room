import { createActor, type SnapshotFrom } from 'xstate';
import { createMessagePublicationMachine } from '@/features/messages/application/message-publication-machine';
import type { MessagePublisher } from '@/features/messages/domain/publication';
import type { RoomMessageSignal } from '@/features/messages/domain/message';
import { maximumMentions, replyRelation, validConversation } from '../domain/conversation';
import { conversationDraft } from '../domain/conversation-draft';

type Publication = SnapshotFrom<ReturnType<typeof createMessagePublicationMachine>>;
type SubmissionIds = { next(): string };
type ComposerSnapshot = {
  readonly publication: Publication;
  readonly text: string;
  readonly mentions: readonly string[];
  readonly reply: RoomMessageSignal | null;
};

/** 一个工作区拥有各房间的草稿和发送事务，面板切换只订阅或取消订阅。 */
export class ConversationWorkspaceStore {
  readonly #rooms = new Map<string, ConversationSessionStore>();
  readonly #publisher: MessagePublisher;
  readonly #ids: SubmissionIds;
  #owners = 0;

  constructor(publisher: MessagePublisher, ids: SubmissionIds) {
    this.#publisher = publisher;
    this.#ids = ids;
  }

  room(roomId: string): ConversationSessionStore {
    let session = this.#rooms.get(roomId);
    if (session === undefined) {
      session = new ConversationSessionStore(this.#publisher, roomId, this.#ids);
      this.#rooms.set(roomId, session);
    }
    return session;
  }

  readonly retain = (): (() => void) => {
    this.#owners += 1;
    let released = false;
    return () => {
      if (released) return;
      released = true;
      this.#owners -= 1;
      // React StrictMode 会在同一轮重连订阅，不能因此中止尚未确认的发送。
      queueMicrotask(() => {
        if (this.#owners !== 0) return;
        for (const session of this.#rooms.values()) session.dispose();
        this.#rooms.clear();
      });
    };
  };
}

class ConversationSessionStore {
  readonly #actor;
  readonly #roomId: string;
  readonly #ids: SubmissionIds;
  readonly #listeners = new Set<() => void>();
  #detach: (() => void) | null = null;
  #snapshot: ComposerSnapshot;

  constructor(publisher: MessagePublisher, roomId: string, ids: SubmissionIds) {
    this.#actor = createActor(createMessagePublicationMachine(publisher));
    this.#roomId = roomId;
    this.#ids = ids;
    this.#snapshot = Object.freeze({
      publication: this.#actor.getSnapshot(),
      text: '',
      mentions: [],
      reply: null,
    });
  }

  readonly getSnapshot = (): ComposerSnapshot => this.#snapshot;
  readonly subscribe = (listener: () => void): (() => void) => {
    this.#listeners.add(listener);
    if (this.#detach === null) {
      const subscription = this.#actor.subscribe((publication) => {
        const justPublished =
          publication.matches('published') && !this.#snapshot.publication.matches('published');
        this.#update({
          ...this.#snapshot,
          ...(justPublished ? { text: '', mentions: [], reply: null } : {}),
          publication,
        });
      });
      this.#detach = () => {
        subscription.unsubscribe();
      };
      this.#actor.start();
      this.#actor.send({ type: 'OPEN', roomId: this.#roomId });
    }
    return () => {
      this.#listeners.delete(listener);
    };
  };

  get editable(): boolean {
    return (
      this.#snapshot.publication.matches('ready') || this.#snapshot.publication.matches('published')
    );
  }

  readonly changeText = (text: string): void => {
    if (this.#prepareEdit()) this.#update({ ...this.#snapshot, text });
  };
  readonly mention = (id: string): void => {
    if (
      this.#snapshot.mentions.includes(id) ||
      this.#snapshot.mentions.length >= maximumMentions ||
      !this.#prepareEdit()
    )
      return;
    this.#update({ ...this.#snapshot, mentions: [...this.#snapshot.mentions, id] });
  };
  readonly respond = (reply: RoomMessageSignal): void => {
    if (reply.roomId !== this.#roomId || reply.lifecycle !== 'active' || !this.#prepareEdit())
      return;
    this.#update({ ...this.#snapshot, reply, mentions: [reply.actor.matrixUserId] });
  };
  readonly removeMention = (id: string): void => {
    if (this.#prepareEdit())
      this.#update({
        ...this.#snapshot,
        mentions: this.#snapshot.mentions.filter((value) => value !== id),
      });
  };
  readonly cancelReply = (): void => {
    if (this.#prepareEdit()) this.#update({ ...this.#snapshot, reply: null });
  };
  readonly submit = (): void => {
    if (!validConversation(this.#snapshot) || !this.#prepareEdit()) return;
    const { text, mentions, reply } = this.#snapshot;
    this.#actor.send({
      type: 'SUBMIT',
      request: {
        ...conversationDraft(text, mentions, reply === null ? undefined : replyRelation(reply)),
        roomId: this.#roomId,
        submissionId: this.#ids.next(),
      },
    });
  };
  readonly retry = (): void => {
    this.#actor.send({ type: 'RETRY' });
  };
  readonly reconcile = (): void => {
    this.#actor.send({ type: 'RECONCILE' });
  };
  readonly retryIdentity = (): void => {
    this.#actor.send({ type: 'RETRY_IDENTITY' });
  };
  readonly edit = (): void => {
    this.#actor.send({ type: 'CLOSE' });
    this.#actor.send({ type: 'OPEN', roomId: this.#roomId });
  };

  dispose(): void {
    this.#detach?.();
    this.#actor.stop();
    this.#listeners.clear();
  }

  #prepareEdit(): boolean {
    if (!this.editable) return false;
    if (this.#snapshot.publication.matches('published')) this.#actor.send({ type: 'RESET' });
    return true;
  }

  #update(snapshot: ComposerSnapshot): void {
    this.#snapshot = Object.freeze({ ...snapshot, mentions: Object.freeze(snapshot.mentions) });
    for (const listener of this.#listeners) listener();
  }
}
