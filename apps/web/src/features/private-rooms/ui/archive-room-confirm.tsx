import { Button } from '@agent-room/ui-system';
import { Archive, LoaderCircle, X } from 'lucide-react';
import { useEffect, useId, useRef } from 'react';
import { useTranslation } from 'react-i18next';

import './archive-room-confirm.css';

export type ArchiveRoomConfirmProps = {
  readonly name: string;
  readonly pending: boolean;
  readonly onCancel: () => void;
  readonly onConfirm: () => void;
};

/**
 * 归档前在原处确认一次（`specs/cleanup-without-reauth/design.md`）：归档以后谁都不能再说话，也撤不回来。
 * 归档不要最近登录过，这一步防的是手滑。房间设置和“房间”页的卡片都用它；打开时焦点落在“取消”上，
 * 回车不会直接归档。
 */
export function ArchiveRoomConfirm({
  name,
  pending,
  onCancel,
  onConfirm,
}: ArchiveRoomConfirmProps) {
  const { t } = useTranslation();
  const titleId = useId();
  const cancel = useRef<HTMLButtonElement>(null);
  useEffect(() => {
    cancel.current?.focus();
  }, []);
  return (
    <div aria-labelledby={titleId} className="archive-room-confirm" role="group">
      <div className="archive-room-confirm__text">
        <strong id={titleId}>{t('privateRooms.archiveConfirm.title', { name })}</strong>
        <p>{t('privateRooms.archiveConfirm.detail')}</p>
      </div>
      <div className="archive-room-confirm__actions">
        <Button
          disabled={pending}
          icon={<X aria-hidden="true" />}
          onClick={onCancel}
          ref={cancel}
          size="compact"
          tone="ghost"
        >
          {t('privateRooms.action.cancel')}
        </Button>
        <Button
          disabled={pending}
          icon={
            pending ? (
              <LoaderCircle aria-hidden="true" className="archive-room-confirm__spin" />
            ) : (
              <Archive aria-hidden="true" />
            )
          }
          onClick={onConfirm}
          size="compact"
          tone="alert"
        >
          {pending
            ? t('privateRooms.archiveConfirm.archiving')
            : t('privateRooms.archiveConfirm.confirm')}
        </Button>
      </div>
    </div>
  );
}
