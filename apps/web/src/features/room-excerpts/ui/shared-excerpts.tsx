import { Button, CopyBlock, Spinner } from '@agent-room/ui-system';
import { useMutation, useQueryClient } from '@tanstack/react-query';
import { RefreshCw, Trash2 } from 'lucide-react';
import { useEffect, useId, useRef, useState } from 'react';
import { useTranslation } from 'react-i18next';

import type { PrivateRoomFailure } from '@/features/private-rooms/domain/private-room';
import {
  roomExcerptsQueryKey,
  useRoomExcerpts,
} from '@/features/room-excerpts/data/room-excerpt-queries';
import type {
  RoomExcerpt,
  RoomExcerptGateway,
} from '@/features/room-excerpts/domain/room-excerpts';
import { formatDayAndTime } from '@/shared/i18n/formatters';

import { ExcerptFailureNotice } from './excerpt-failure';
import './room-excerpts.css';

export type SharedExcerptsProps = {
  readonly gateway: RoomExcerptGateway;
  readonly catalogId: string;
  /** 你是房主或管理员：服务器给的是这间房的全部，标题和说明跟着换。 */
  readonly manages: boolean;
  /** 房间此刻放着实况：一段也没有时说怎么分享；没在放又一段也没有时整节不出来。 */
  readonly live: boolean;
};

/**
 * 房间“分享”对话框里分享过的几段对话（`specs/redesign/project-rooms.md`“分享一段对话”，3P-7b）：几句、哪天、
 * 链接，复制和删掉；删之前在这一行里确认一次。成员只看得到自己的（标题“我分享过的”）；房主和管理员看得到
 * 这间房的全部（标题“这间房分享过的”），只标出哪几段是自己的，看不出别的是谁分享的。
 */
export function SharedExcerpts({ gateway, catalogId, manages, live }: SharedExcerptsProps) {
  const { t } = useTranslation();
  const headingId = useId();
  const heading = useRef<HTMLHeadingElement>(null);
  const query = useRoomExcerpts(gateway, catalogId);
  const [deleted, setDeleted] = useState(false);
  const excerpts = query.data?.ok === true ? query.data.value : null;
  // 不在放实况、又一段也没有：这一节没什么可说的。
  if (excerpts !== null && excerpts.length === 0 && !live && !deleted) return null;
  // 房主和管理员看到的是这间房的全部（别人的也在），标题不能说“我分享过的”。
  const roomWide = manages;

  let content;
  if (query.isPending) {
    content = (
      <p className="room-excerpts__note" role="status">
        <Spinner />
        {t('roomExcerpt.list.loading')}
      </p>
    );
  } else if (excerpts === null) {
    content = (
      <div className="room-excerpts__state">
        <p className="room-excerpts__note">{t('roomExcerpt.list.failed')}</p>
        <Button
          icon={<RefreshCw aria-hidden="true" />}
          onClick={() => void query.refetch()}
          size="compact"
          tone="quiet"
        >
          {t('roomExcerpt.list.retry')}
        </Button>
      </div>
    );
  } else if (excerpts.length === 0) {
    content = <p className="room-excerpts__note">{t('roomExcerpt.list.empty')}</p>;
  } else {
    content = (
      <ul className="room-excerpts__list">
        {excerpts.map((excerpt) => (
          <SharedExcerptRow
            catalogId={catalogId}
            excerpt={excerpt}
            gateway={gateway}
            key={excerpt.code}
            onDeleted={() => {
              setDeleted(true);
              heading.current?.focus();
            }}
            roomWide={roomWide}
          />
        ))}
      </ul>
    );
  }

  return (
    <section aria-labelledby={headingId} className="room-excerpts">
      <h3 id={headingId} ref={heading} tabIndex={-1}>
        {t(roomWide ? 'roomExcerpt.list.title.room' : 'roomExcerpt.list.title.mine')}
      </h3>
      <p className="room-excerpts__note">
        {t(roomWide ? 'roomExcerpt.list.detail.room' : 'roomExcerpt.list.detail.mine')}
      </p>
      {content}
      <p aria-live="polite" className="sr-only" role="status">
        {deleted ? t('roomExcerpt.list.deleted') : ''}
      </p>
    </section>
  );
}

function SharedExcerptRow({
  gateway,
  catalogId,
  excerpt,
  roomWide,
  onDeleted,
}: {
  readonly gateway: RoomExcerptGateway;
  readonly catalogId: string;
  readonly excerpt: RoomExcerpt;
  readonly roomWide: boolean;
  readonly onDeleted: () => void;
}) {
  const { t, i18n } = useTranslation();
  const queryClient = useQueryClient();
  const labelId = useId();
  const [confirming, setConfirming] = useState(false);
  const keep = useRef<HTMLButtonElement>(null);
  const deleteButton = useRef<HTMLButtonElement>(null);
  const removal = useMutation({
    mutationFn: async () => await gateway.remove(catalogId, excerpt.code),
    onSuccess: (result) => {
      // 删掉了、或者早就不在了：都从名单上拿掉，重读一次。
      if (result.ok || result.error.code === 'room_excerpt.not_found') {
        setConfirming(false);
        if (result.ok) onDeleted();
        void queryClient.invalidateQueries({ queryKey: roomExcerptsQueryKey(catalogId) });
      }
    },
  });
  const failure: PrivateRoomFailure | null =
    removal.data?.ok === false
      ? removal.data.error
      : removal.isError
        ? { code: 'private_room.unreachable', retryable: true }
        : null;
  // 出来确认时焦点落在“不删了”上（删不能撤回）；收回去时回到“删掉”。
  const wasConfirming = useRef(false);
  useEffect(() => {
    if (confirming) keep.current?.focus();
    else if (wasConfirming.current) deleteButton.current?.focus();
    wasConfirming.current = confirming;
  }, [confirming]);

  return (
    <li className="room-excerpts__item">
      <div aria-labelledby={labelId} className="room-excerpts__group" role="group">
        <p className="room-excerpts__label" id={labelId}>
          <span>
            {t('roomExcerpt.list.item', {
              count: excerpt.messageCount,
              date: formatDayAndTime(
                excerpt.firstMessageAtUnixMs,
                Date.now(),
                i18n.resolvedLanguage,
              ),
            })}
          </span>
          {roomWide && excerpt.mine ? (
            <span className="room-excerpts__mine">{t('roomExcerpt.list.mine')}</span>
          ) : null}
        </p>
        {excerpt.url === null ? (
          <p className="room-excerpts__note">{t('roomExcerpt.list.noLink')}</p>
        ) : (
          <a className="room-excerpts__url" href={excerpt.url} rel="noreferrer" target="_blank">
            {excerpt.url.replace(/^https?:\/\//u, '')}
          </a>
        )}
        {confirming ? (
          <div className="room-excerpts__confirm">
            <p>{t('roomExcerpt.list.confirm')}</p>
            <div className="room-excerpts__actions">
              <Button
                disabled={removal.isPending}
                onClick={() => {
                  removal.mutate();
                }}
                size="compact"
                tone="alert"
              >
                {removal.isPending
                  ? t('roomExcerpt.list.deleting')
                  : t('roomExcerpt.list.confirmYes')}
              </Button>
              <Button
                disabled={removal.isPending}
                onClick={() => {
                  removal.reset();
                  setConfirming(false);
                }}
                ref={keep}
                size="compact"
                tone="ghost"
              >
                {t('roomExcerpt.list.confirmNo')}
              </Button>
            </div>
          </div>
        ) : (
          <div className="room-excerpts__actions">
            {excerpt.url === null ? null : (
              <CopyBlock
                copiedLabel={t('roomShare.copied')}
                copyLabel={t('roomExcerpt.list.copy')}
                failedLabel={t('roomShare.copyFailed')}
                hideText
                size="compact"
                text={excerpt.url}
                tone="ghost"
              />
            )}
            <Button
              icon={<Trash2 aria-hidden="true" />}
              onClick={() => {
                removal.reset();
                setConfirming(true);
              }}
              ref={deleteButton}
              size="compact"
              tone="ghost"
            >
              {t('roomExcerpt.list.delete')}
            </Button>
          </div>
        )}
        {failure === null || failure.code === 'room_excerpt.not_found' ? null : (
          <ExcerptFailureNotice action="delete" failure={failure} />
        )}
      </div>
    </li>
  );
}
