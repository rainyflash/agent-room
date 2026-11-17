import { Button } from '@agent-room/ui-system';
import { Share2 } from 'lucide-react';
import { useTranslation } from 'react-i18next';

import { EXCERPT_MAX_MESSAGES } from '@/features/room-excerpts/domain/room-excerpts';

import './room-excerpts.css';

/**
 * 选一段对话时对话上面那一条（`specs/redesign/project-rooms.md`“分享一段对话”）：左边“已选 6 句”和怎么接着选，
 * 右边“分享”“取消”。选了几句、够不着时读屏都会念出来；Esc 也是取消。
 */
export function ExcerptSelectionBar({
  count,
  tooLong,
  sharing,
  onShare,
  onCancel,
}: {
  readonly count: number;
  readonly tooLong: boolean;
  /** 正在向控制面要链接：“分享”先灰着。 */
  readonly sharing: boolean;
  readonly onShare: () => void;
  readonly onCancel: () => void;
}) {
  const { t } = useTranslation();
  return (
    <section aria-label={t('roomExcerpt.bar.label')} className="excerpt-bar">
      <p aria-live="polite" className="excerpt-bar__status" role="status">
        <strong>{t('roomExcerpt.bar.count', { count })}</strong>
        <span data-too-long={tooLong ? 'true' : undefined}>
          {tooLong
            ? t('roomExcerpt.bar.tooLong', { max: EXCERPT_MAX_MESSAGES })
            : t('roomExcerpt.bar.hint', { max: EXCERPT_MAX_MESSAGES })}
        </span>
      </p>
      <div className="excerpt-bar__actions">
        <Button
          disabled={sharing}
          icon={<Share2 aria-hidden="true" />}
          onClick={onShare}
          size="compact"
          tone="primary"
        >
          {t('roomExcerpt.bar.share')}
        </Button>
        <Button onClick={onCancel} size="compact" tone="ghost">
          {t('roomExcerpt.bar.cancel')}
        </Button>
      </div>
    </section>
  );
}
