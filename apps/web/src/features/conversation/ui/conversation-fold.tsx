import { ChevronDown } from 'lucide-react';
import type { ReactNode } from 'react';
import { useTranslation } from 'react-i18next';

import { AgentAvatar } from '@/features/lobby/ui/avatars';
import type { ConversationFold } from '../domain/conversation-folds';

/** 折起来的那一行最多画几个头像。 */
const foldAvatars = 3;

/**
 * Agent 之间折起来的一串占的那一行：几个头像、“甲和乙之间又说了 6 句”。点一下展开，再点收起。
 * 展开以后这一行留着，那几句跟在它下面。
 */
export function ConversationFoldRow({
  fold,
  open,
  onToggle,
  separator,
}: {
  readonly fold: ConversationFold;
  readonly open: boolean;
  readonly onToggle: () => void;
  /** 这一串是当天的第一项时，上面那一行日期。 */
  readonly separator: ReactNode;
}) {
  const { t } = useTranslation();
  const [first, second] = fold.speakers;
  const count = fold.messages.length;
  const label =
    fold.speakers.length === 1 && first !== undefined
      ? t('conversation.fold.one', { count, name: first.displayName })
      : fold.speakers.length === 2 && first !== undefined && second !== undefined
        ? t('conversation.fold.two', {
            count,
            first: first.displayName,
            second: second.displayName,
          })
        : t('conversation.fold.many', { count, agents: fold.speakers.length });
  return (
    <div
      className="conversation-fold"
      // 记着读到哪儿、要跳到哪条消息时，折着的那几句都算在这一行上。
      data-conversation-fold={fold.messages.map((message) => message.messageId).join(' ')}
      {...(open ? {} : { 'data-conversation-message-id': fold.messages[0]?.messageId })}
    >
      {separator}
      <button
        type="button"
        className="conversation-fold__toggle"
        aria-expanded={open}
        onClick={onToggle}
      >
        <span className="conversation-fold__avatars" aria-hidden="true">
          {fold.speakers.slice(0, foldAvatars).map((speaker) => (
            <AgentAvatar id={speaker.agentId} key={speaker.agentId} />
          ))}
        </span>
        <span className="conversation-fold__label">{label}</span>
        <ChevronDown aria-hidden="true" />
      </button>
    </div>
  );
}
