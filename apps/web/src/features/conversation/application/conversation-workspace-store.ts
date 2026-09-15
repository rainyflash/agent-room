import { createActor, type SnapshotFrom } from 'xstate';
import { createMessagePublicationMachine } from '@/features/messages/application/message-publication-machine';
import type { MessagePublisher } from '@/features/messages/domain/publication';
import type { RoomMessageSignal } from '@/features/messages/domain/message';
import { maximumMentions, replyRelation, validConversation } from '../domain/conversation';
import { conversationDraft } from '../domain/conversation-draft';
import type { ConversationReply, ConversationStorage } from '../domain/conversation-storage';
import { registerUpdateGuard } from '@/features/updates/application/update-readiness';
import {
  attachmentIssue,
  type AttachmentFailure,
  type AttachmentReference,
  type AttachmentState,
  type ConversationAttachment,
  type ConversationAttachmentStorage,
} from '../domain/conversation-attachment';

type Publication = SnapshotFrom<ReturnType<typeof createMessagePublicationMachine>>;
type SubmissionIds = { next(): string };
type ComposerSnapshot = {
  readonly attachment: AttachmentState;
  readonly attachmentFailure: AttachmentFailure | null;
  readonly publication: Publication;
  readonly text: string;
  readonly mentions: readonly string[];
  readonly reply: ConversationReply | null;
  readonly draftPersistence: 'saved' | 'unavailable' | 'disabled';
};

/** 一个工作区拥有各房间的草稿和发送事务，面板切换只订阅或取消订阅。 */
export class ConversationWorkspaceStore {
  readonly #rooms = new Map<string, ConversationSessionStore>();
  readonly #publisher: MessagePublisher;
  readonly #ids: SubmissionIds;
  #owners = 0;
  #detachGuard: (() => void) | null = null;

  constructor(
    publisher: MessagePublisher,
    ids: SubmissionIds,
    private readonly storage?: ConversationStorage,
    private readonly attachments?: ConversationAttachmentStorage,
  ) {
    this.#publisher = publisher;
    this.#ids = ids;
  }

  room(roomId: string): ConversationSessionStore {
    let session = this.#rooms.get(roomId);
    if (session === undefined) {
      session = new ConversationSessionStore(
        this.#publisher,
        roomId,
        this.#ids,
        this.storage,
        this.attachments,
      );
      this.#rooms.set(roomId, session);
    }
    return session;
  }

  readonly retain = (): (() => void) => {
    this.#detachGuard ??= registerUpdateGuard(() =>
      [...this.#rooms.values()].every((session) => session.safeToReload),
    );
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
        this.#detachGuard?.();
        this.#detachGuard = null;
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
  #restoring: string | null = null;
  #persistAllowed = true;
  #attachmentGeneration = 0;

  constructor(
    publisher: MessagePublisher,
    roomId: string,
    ids: SubmissionIds,
    private readonly storage?: ConversationStorage,
    private readonly attachments?: ConversationAttachmentStorage,
  ) {
    this.#actor = createActor(createMessagePublicationMachine(publisher));
    this.#roomId = roomId;
    this.#ids = ids;
    const saved = storage?.read(roomId);
    this.#persistAllowed = saved?.ok !== false;
    const draft = saved?.ok ? saved.value : null;
    this.#restoring = draft?.pendingSubmissionId ?? null;
    this.#snapshot = Object.freeze({
      attachment:
        draft?.attachment === undefined
          ? { kind: 'none' as const }
          : { kind: 'loading' as const, reference: draft.attachment },
      attachmentFailure: null,
      publication: this.#actor.getSnapshot(),
      text: draft?.text ?? '',
      mentions: draft?.mentions ?? [],
      reply: draft?.reply ?? null,
      draftPersistence: saved === undefined ? 'disabled' : saved.ok ? 'saved' : 'unavailable',
    });
  }

  readonly getSnapshot = (): ComposerSnapshot => this.#snapshot;
  readonly subscribe = (listener: () => void): (() => void) => {
    this.#listeners.add(listener);
    if (this.#detach === null) {
      const subscription = this.#actor.subscribe((publication) => {
        const previousAttachment = this.#snapshot.attachment;
        const justPublished =
          publication.matches('published') && !this.#snapshot.publication.matches('published');
        this.#update({
          ...this.#snapshot,
          ...(justPublished
            ? {
                text: '',
                mentions: [],
                reply: null,
                attachment: { kind: 'none' as const },
                attachmentFailure: null,
              }
            : {}),
          publication,
        });
        this.#restoreSubmission();
        if (
          justPublished &&
          previousAttachment.kind !== 'none' &&
          this.#snapshot.draftPersistence === 'saved'
        )
          void this.#discardAttachment(previousAttachment.reference);
      });
      this.#detach = () => {
        subscription.unsubscribe();
      };
      this.#actor.start();
      this.#actor.send({ type: 'OPEN', roomId: this.#roomId });
      if (this.#snapshot.attachment.kind === 'loading') void this.#restoreAttachment();
    }
    return () => {
      this.#listeners.delete(listener);
    };
  };

  #restoreSubmission(): void {
    if (
      this.#restoring !== null &&
      this.#snapshot.publication.matches('ready') &&
      (this.#snapshot.attachment.kind === 'none' || this.#snapshot.attachment.kind === 'ready')
    ) {
      const submissionId = this.#restoring;
      this.#restoring = null;
      const { text, mentions, reply } = this.#snapshot;
      this.#actor.send({
        type: 'RESTORE',
        request: {
          ...conversationDraft(
            text,
            mentions,
            reply === null ? undefined : replyRelation(reply),
            this.#snapshot.attachment.kind === 'ready' ? this.#snapshot.attachment.file : undefined,
          ),
          roomId: this.#roomId,
          submissionId,
        },
      });
    }
  }

  get editable(): boolean {
    return (
      this.#snapshot.publication.matches('ready') || this.#snapshot.publication.matches('published')
    );
  }

  get safeToReload(): boolean {
    if (this.#snapshot.attachment.kind === 'ready')
      return this.attachments !== undefined && this.#snapshot.draftPersistence === 'saved';
    return (
      this.#snapshot.attachment.kind !== 'loading' &&
      (this.#snapshot.draftPersistence === 'saved' ||
        (this.#snapshot.text === '' &&
          this.#snapshot.mentions.length === 0 &&
          this.#snapshot.reply === null &&
          this.#snapshot.publication.context.request === null))
    );
  }

  get valid(): boolean {
    const { text, mentions, attachment } = this.#snapshot;
    return (
      (attachment.kind === 'none' || attachment.kind === 'ready') &&
      validConversation({
        text: text.trim().length === 0 && attachment.kind === 'ready' ? attachment.file.name : text,
        mentions,
      })
    );
  }

  readonly attach = async (file: ConversationAttachment): Promise<void> => {
    if (!this.#prepareEdit()) return;
    const issue = attachmentIssue(file);
    if (issue !== null) {
      this.#update({ ...this.#snapshot, attachmentFailure: issue });
      return;
    }
    const generation = ++this.#attachmentGeneration;
    const reference = { key: this.#ids.next(), name: file.name };
    const previous = this.#snapshot.attachment;
    this.#update({
      ...this.#snapshot,
      attachment: { kind: 'loading', reference },
      attachmentFailure: null,
    });
    const saved = await this.attachments?.write(reference.key, file);
    if (generation !== this.#attachmentGeneration) {
      // Removal can finish before the pending write. Delete again after the write,
      // but keep a draft still referenced when the workspace is merely unmounted.
      const current = this.#snapshot.attachment;
      if (
        saved?.ok &&
        this.#snapshot.draftPersistence === 'saved' &&
        (current.kind === 'none' || current.reference.key !== reference.key)
      )
        await this.#discardAttachment(reference);
      return;
    }
    if (saved?.ok === false) {
      this.#update({ ...this.#snapshot, attachment: previous, attachmentFailure: saved.error });
      return;
    }
    this.#update({ ...this.#snapshot, attachment: { kind: 'ready', reference, file } });
    if (previous.kind !== 'none' && this.#snapshot.draftPersistence === 'saved')
      void this.#discardAttachment(previous.reference);
    // A recovered interrupted submission must be reconciled with its original identity.
    this.#restoreSubmission();
  };
  readonly removeAttachment = (): void => {
    if (!this.#prepareEdit() || this.#restoring !== null) return;
    this.#attachmentGeneration += 1;
    const previous = this.#snapshot.attachment;
    this.#update({ ...this.#snapshot, attachment: { kind: 'none' }, attachmentFailure: null });
    if (previous.kind !== 'none' && this.#snapshot.draftPersistence === 'saved')
      void this.#discardAttachment(previous.reference);
  };
  async #discardAttachment(reference: AttachmentReference): Promise<void> {
    const removed = await this.attachments?.remove(reference.key);
    if (removed?.ok === false)
      this.#update({ ...this.#snapshot, attachmentFailure: removed.error });
  }
  async #restoreAttachment(): Promise<void> {
    const state = this.#snapshot.attachment;
    if (state.kind !== 'loading') return;
    const generation = ++this.#attachmentGeneration;
    const loaded = await this.attachments?.read(state.reference.key);
    if (generation !== this.#attachmentGeneration) return;
    const attachment: AttachmentState =
      loaded?.ok && loaded.value !== null
        ? { kind: 'ready', reference: state.reference, file: loaded.value }
        : { kind: 'missing', reference: state.reference };
    this.#update({ ...this.#snapshot, attachment });
    this.#restoreSubmission();
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
    this.#update({
      ...this.#snapshot,
      reply: {
        messageId: reply.messageId,
        actor: {
          displayName: reply.actor.displayName,
          matrixUserId: reply.actor.matrixUserId,
        },
      },
      mentions: [reply.actor.matrixUserId],
    });
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
    if (!this.valid || !this.#prepareEdit()) return;
    const { text, mentions, reply } = this.#snapshot;
    this.#actor.send({
      type: 'SUBMIT',
      request: {
        ...conversationDraft(
          text,
          mentions,
          reply === null ? undefined : replyRelation(reply),
          this.#snapshot.attachment.kind === 'ready' ? this.#snapshot.attachment.file : undefined,
        ),
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
    this.#attachmentGeneration += 1;
    this.#detach?.();
    this.#actor.stop();
    this.#listeners.clear();
  }

  #prepareEdit(): boolean {
    if (!this.editable) return false;
    this.#persistAllowed = true;
    if (this.#snapshot.publication.matches('published')) this.#actor.send({ type: 'RESET' });
    return true;
  }

  #update(snapshot: ComposerSnapshot): void {
    const publication = snapshot.publication;
    const pendingSubmissionId =
      this.#restoring ??
      (publication.matches('published')
        ? null
        : (publication.context.request?.submissionId ?? null));
    const saved = this.#persistAllowed
      ? this.storage?.write(this.#roomId, {
          version: 1,
          text: snapshot.text,
          mentions: [...snapshot.mentions],
          reply: snapshot.reply,
          pendingSubmissionId,
          ...(snapshot.attachment.kind === 'none'
            ? {}
            : { attachment: snapshot.attachment.reference }),
        })
      : undefined;
    this.#snapshot = Object.freeze({
      ...snapshot,
      mentions: Object.freeze(snapshot.mentions),
      draftPersistence: !this.#persistAllowed
        ? 'unavailable'
        : saved === undefined
          ? 'disabled'
          : saved.ok
            ? 'saved'
            : 'unavailable',
    });
    for (const listener of this.#listeners) listener();
  }
}
