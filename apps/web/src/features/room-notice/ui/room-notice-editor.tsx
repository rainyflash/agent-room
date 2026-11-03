import { Banner, Button, Field } from '@agent-room/ui-system';
import { useMutation } from '@tanstack/react-query';
import { useState } from 'react';
import { useTranslation } from 'react-i18next';

import {
  characterCount,
  normalizeRoomNotice,
  ROOM_NOTICE_MAX_CHARACTERS,
  roomNoticeProblem,
  type RoomNotice,
  type RoomNoticeGateway,
  type WrittenRoomNotice,
} from '@/features/room-notice/domain/room-notice';
import { formatNumber } from '@/shared/i18n/formatters';

import './room-notice-editor.css';

export type RoomNoticeEditorProps = {
  readonly catalogId: string;
  /** 房间状态里此刻的公告；没有就是还没写过或撤掉了。 */
  readonly notice: RoomNotice | undefined;
  readonly gateway: RoomNoticeGateway;
};

/**
 * 房间设置里的“公告”（房主和管理员）：写一段纯文本，保存就盖掉旧的，撤掉就是清空。写进去以后要等同步回来
 * 房间状态才变，这之间照刚写进去的那一版显示。
 */
export function RoomNoticeEditor({ catalogId, notice, gateway }: RoomNoticeEditorProps) {
  const { t, i18n } = useTranslation();
  // 改了才记下；没改时跟着房间状态。
  const [draft, setDraft] = useState<string | null>(null);
  const [written, setWritten] = useState<WrittenRoomNotice | null>(null);
  const current =
    written !== null && (notice === undefined || notice.updatedAtUnixMs < written.updatedAtUnixMs)
      ? written.text
      : (notice?.text ?? '');
  const shown = draft ?? current;
  const outgoing = normalizeRoomNotice(shown);
  const changed = outgoing !== current;
  const problem = roomNoticeProblem(shown);
  const update = useMutation({
    mutationFn: async (text: string) => await gateway.updateNotice(catalogId, text),
    onSuccess: (result) => {
      if (!result.ok) return;
      setWritten(result.value);
      setDraft(null);
    },
  });
  const save = (text: string): void => {
    update.reset();
    update.mutate(text);
  };

  return (
    <div className="room-notice-editor">
      <Field
        error={problem === null ? undefined : t(problemKey[problem])}
        hint={t('roomNotice.editor.hint')}
        label={t('roomNotice.editor.label')}
      >
        <textarea
          disabled={update.isPending}
          onChange={(event) => {
            update.reset();
            setDraft(event.target.value);
          }}
          rows={6}
          value={shown}
        />
      </Field>
      <p className="room-notice-editor__meta">
        <span>{t('roomNotice.editor.readers')}</span>
        <span className="room-notice-editor__count">
          {t('roomNotice.editor.used', {
            max: formatNumber(ROOM_NOTICE_MAX_CHARACTERS, i18n.language),
            used: formatNumber(characterCount(outgoing), i18n.language),
          })}
        </span>
      </p>
      <div className="room-notice-editor__actions">
        <Button
          disabled={!changed || problem !== null || update.isPending}
          onClick={() => {
            save(outgoing);
          }}
          tone="primary"
        >
          {update.isPending ? t('roomNotice.editor.saving') : t('roomNotice.editor.save')}
        </Button>
        {draft !== null && changed ? (
          <Button
            disabled={update.isPending}
            onClick={() => {
              update.reset();
              setDraft(null);
            }}
            tone="ghost"
          >
            {t('roomNotice.editor.discard')}
          </Button>
        ) : null}
        {current === '' ? null : (
          <Button
            disabled={update.isPending}
            onClick={() => {
              setDraft(null);
              save('');
            }}
            tone="ghost"
          >
            {t('roomNotice.editor.remove')}
          </Button>
        )}
      </div>
      {update.data?.ok === true ? (
        <p className="room-notice-editor__status" role="status">
          {t(
            update.data.value.text === '' ? 'roomNotice.editor.removed' : 'roomNotice.editor.saved',
          )}
        </p>
      ) : null}
      {update.data?.ok === false ? (
        <Banner tone="warning">{t(failureKey(update.data.error.code))}</Banner>
      ) : null}
    </div>
  );
}

const problemKey = {
  control_character: 'roomNotice.editor.controlCharacter',
  too_long: 'roomNotice.editor.tooLong',
} as const;

type FailureKey =
  | 'roomNotice.failure.archived'
  | 'roomNotice.failure.forbidden'
  | 'roomNotice.failure.other'
  | 'roomNotice.failure.rateLimited'
  | 'roomNotice.failure.unavailable';

function failureKey(code: string): FailureKey {
  switch (code) {
    case 'room_notice.rate_limited':
      return 'roomNotice.failure.rateLimited';
    case 'room_notice.dependency_unavailable':
    case 'private_room.unreachable':
      return 'roomNotice.failure.unavailable';
    case 'room_notice.forbidden':
      return 'roomNotice.failure.forbidden';
    case 'room_notice.conflict':
      return 'roomNotice.failure.archived';
    default:
      return 'roomNotice.failure.other';
  }
}
