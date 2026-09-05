import { conversationDraft } from '@/features/conversation/domain/conversation-draft';
import { useMachine } from '@xstate/react';
import { useEffect, useMemo, useState } from 'react';
import { createMessagePublicationMachine } from '@/features/messages/application/message-publication-machine';
import {
  BrowserSubmissionIdFactory,
  type MessageSubmissionIdFactory,
} from '@/features/messages/adapters/browser-submission-id-factory';
import type { MessagePublisher } from '@/features/messages/domain/publication';
import type { RoomMessageSignal } from '@/features/messages/domain/message';
import {
  maximumMentions,
  replyRelation,
  validConversation,
} from '@/features/conversation/domain/conversation';

const browserIds = new BrowserSubmissionIdFactory();

export function useConversationComposer(
  publisher: MessagePublisher,
  roomId: string,
  submissionIds: MessageSubmissionIdFactory = browserIds,
) {
  const machine = useMemo(() => createMessagePublicationMachine(publisher), [publisher]);
  const [publication, send] = useMachine(machine);
  const [text, setText] = useState('');
  const [mentions, setMentions] = useState<readonly string[]>([]);
  const [reply, setReply] = useState<RoomMessageSignal | null>(null);

  useEffect(() => {
    send({ type: 'OPEN', roomId });
  }, [roomId, send]);
  useEffect(() => {
    if (publication.matches('published')) {
      setText('');
      setMentions([]);
      setReply(null);
    }
  }, [publication]);

  const editable = publication.matches('ready') || publication.matches('published');
  const changeText = (value: string): void => {
    if (!editable) return;
    if (publication.matches('published')) send({ type: 'RESET' });
    setText(value);
  };
  const mention = (id: string): void => {
    if (!editable || mentions.includes(id) || mentions.length >= maximumMentions) return;
    if (publication.matches('published')) send({ type: 'RESET' });
    setMentions((current) => [...current, id]);
  };
  const respond = (message: RoomMessageSignal): void => {
    if (!editable || message.roomId !== roomId || message.lifecycle !== 'active') return;
    if (publication.matches('published')) send({ type: 'RESET' });
    setReply(message);
    setMentions([message.actor.matrixUserId]);
  };
  return {
    editable,
    mentions,
    publication,
    reply,
    text,
    valid: validConversation({ text, mentions }),
    changeText,
    mention,
    respond,
    removeMention: (id: string): void => {
      if (editable) setMentions((current) => current.filter((value) => value !== id));
    },
    cancelReply: (): void => {
      if (editable) setReply(null);
    },
    submit: (): void => {
      if (!editable || !validConversation({ text, mentions })) return;
      if (publication.matches('published')) send({ type: 'RESET' });
      send({
        type: 'SUBMIT',
        request: {
          ...conversationDraft(text, mentions, reply === null ? undefined : replyRelation(reply)),
          roomId,
          submissionId: submissionIds.next(),
        },
      });
    },
    retry: (): void => {
      send({ type: 'RETRY' });
    },
    reconcile: (): void => {
      send({ type: 'RECONCILE' });
    },
    edit: (): void => {
      send({ type: 'CLOSE' });
      send({ type: 'OPEN', roomId });
    },
    retryIdentity: (): void => {
      send({ type: 'RETRY_IDENTITY' });
    },
  };
}
