import { useMemo } from 'react';
import { useTranslation } from 'react-i18next';
import type { RoomMessageSignal } from '@/features/messages/domain/message';
import {
  conversationActorId,
  emptyConversationFilter,
  validSearchDates,
  type ConversationFilter,
} from '../domain/conversation-search';

export function ConversationSearch({
  messages,
  filter,
  onChange,
  count,
}: {
  readonly messages: readonly RoomMessageSignal[];
  readonly filter: ConversationFilter;
  readonly onChange: (filter: ConversationFilter) => void;
  readonly count: number;
}) {
  const { t } = useTranslation();
  const people = useMemo(
    () =>
      [
        ...new Map(
          messages.map((message) => [conversationActorId(message), message.actor.displayName]),
        ).entries(),
      ].toSorted(
        (left, right) => left[1].localeCompare(right[1]) || left[0].localeCompare(right[0]),
      ),
    [messages],
  );
  return (
    <details className="conversation-search" open={filter.topic !== null || undefined}>
      <summary>
        {t('history.search')}
        {filter.topic === null ? '' : ` · ${t('history.topic')}`}
      </summary>
      <div className="conversation-search__fields">
        <label>
          {t('history.keyword')}
          <input
            type="search"
            value={filter.text}
            maxLength={200}
            onChange={(event) => {
              onChange({ ...filter, text: event.target.value });
            }}
          />
        </label>
        <label>
          {t('history.person')}
          <select
            value={filter.actor}
            onChange={(event) => {
              onChange({ ...filter, actor: event.target.value });
            }}
          >
            <option value="">{t('history.anyone')}</option>
            {people.map(([id, name]) => (
              <option key={id} value={id}>
                {name}
              </option>
            ))}
          </select>
        </label>
        <label>
          {t('history.from')}
          <input
            type="date"
            value={filter.from}
            onChange={(event) => {
              onChange({ ...filter, from: event.target.value });
            }}
          />
        </label>
        <label>
          {t('history.until')}
          <input
            type="date"
            value={filter.until}
            onChange={(event) => {
              onChange({ ...filter, until: event.target.value });
            }}
          />
        </label>
        <button
          type="button"
          onClick={() => {
            onChange(emptyConversationFilter);
          }}
        >
          {t('history.clear')}
        </button>
      </div>
      <p role="status">
        {validSearchDates(filter) ? t('history.results', { count }) : t('history.invalidDates')}
      </p>
      {filter.topic !== null ? <p>{t('history.topicScope')}</p> : null}
    </details>
  );
}
