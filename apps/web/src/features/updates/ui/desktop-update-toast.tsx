import { Button, Details, Toast } from '@agent-room/ui-system';
import { useLocation } from '@tanstack/react-router';
import { Download } from 'lucide-react';
import { useEffect, useState } from 'react';
import { useTranslation } from 'react-i18next';

import { updateProgressLabel } from '@/features/desktop/domain/update-progress';
import { useOptionalDesktopRuntimeController } from '@/features/desktop/ui/desktop-runtime-provider';
import {
  browserStorageOrNull,
  readUpdateSnooze,
  snoozeRemainingMs,
  updateSnoozeMs,
  writeUpdateSnooze,
} from './update-snooze';

/**
 * 桌面端查到新版本：提示栈里一条，点一下就更新并重启。点“稍后”这个版本 24 小时后再提醒，
 * 出了更新的版本马上提醒；“设置”上的提醒点一直在。“设置 → 这台电脑”那一页有完整的更新一节，
 * 那里不放。
 */
export function DesktopUpdateToast() {
  const { t } = useTranslation();
  const desktop = useOptionalDesktopRuntimeController();
  const pathname = useLocation({ select: (location) => location.pathname });
  const [storage] = useState(browserStorageOrNull);
  const [snooze, setSnooze] = useState(() => readUpdateSnooze(storage));
  const [now, setNow] = useState(() => Date.now());
  const update =
    desktop?.available === true && desktop.update?.available === true ? desktop.update : null;
  const installing = desktop?.updateBusy === 'installing';
  const remaining = update === null ? 0 : snoozeRemainingMs(snooze, update.targetVersion, now);
  // 应用一开好几天：到点就再提醒，不等别的东西让页面重画。
  useEffect(() => {
    if (remaining === 0) return undefined;
    const timer = window.setTimeout(() => {
      setNow(Date.now());
    }, remaining);
    return () => {
      window.clearTimeout(timer);
    };
  }, [remaining]);
  // 从托盘菜单开始了更新：之前点的“稍后”作废，进度和结果都在这里说。
  useEffect(() => {
    if (!installing || snooze === null) return;
    writeUpdateSnooze(storage, null);
    setSnooze(null);
  }, [installing, snooze, storage]);
  if (
    desktop === null ||
    update === null ||
    pathname === '/settings/this-computer' ||
    (remaining > 0 && !installing)
  ) {
    return null;
  }
  const translocated = desktop.snapshot?.appTranslocated === true;
  const failure = desktop.updateFailure ?? null;
  const later = () => {
    const next = { version: update.targetVersion, untilUnixMs: Date.now() + updateSnoozeMs };
    writeUpdateSnooze(storage, next);
    setSnooze(next);
    setNow(Date.now());
  };
  return (
    <Toast
      // 应用在只读位置运行时装不上，不给按钮，只说怎么办。
      {...(translocated
        ? {}
        : {
            action: (
              <Button
                disabled={desktop.updateBusy != null}
                icon={<Download aria-hidden="true" />}
                onClick={() => void desktop.installUpdate()}
                size="compact"
              >
                {installing
                  ? updateProgressLabel(t, desktop.updateProgress ?? null)
                  : t('toasts.update.install')}
              </Button>
            ),
          })}
      dismissLabel={t('toasts.update.later')}
      {...(installing ? {} : { onDismiss: later })}
      title={t('toasts.update.title', { version: update.targetVersion })}
    >
      {translocated ? <p>{t('desktop.update.translocated')}</p> : null}
      {failure === null ? null : (
        <>
          <p>
            {t(
              failure.code === 'desktop.update.draft_unsaved'
                ? 'conversation.draftUnavailable'
                : 'toasts.update.failed',
            )}
          </p>
          <Details summary={t('entry.details')}>
            <code>{failure.code}</code>
          </Details>
        </>
      )}
    </Toast>
  );
}
