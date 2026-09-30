import { Toast } from '@agent-room/ui-system';
import { Link, useLocation } from '@tanstack/react-router';
import { useState } from 'react';
import { useTranslation } from 'react-i18next';

import { useOptionalDesktopRuntimeController } from '@/features/desktop/ui/desktop-runtime-provider';

/**
 * 桌面端启动后自动查到新版本：提示栈里一条，去“设置 → 这台电脑”安装。点“稍后”这个版本就不再
 * 提醒；“设置”上的提醒点还在。
 */
export function DesktopUpdateToast() {
  const { t } = useTranslation();
  const desktop = useOptionalDesktopRuntimeController();
  const pathname = useLocation({ select: (location) => location.pathname });
  const [dismissed, setDismissed] = useState<string | null>(null);
  const update =
    desktop?.available === true && desktop.update?.available === true ? desktop.update : null;
  if (
    update === null ||
    dismissed === update.targetVersion ||
    pathname === '/settings/this-computer'
  ) {
    return null;
  }
  return (
    <Toast
      action={
        <Link
          className="ar-button ar-button--compact ar-button--primary"
          params={{ section: 'this-computer' }}
          to="/settings/$section"
        >
          {t('toasts.update.open')}
        </Link>
      }
      dismissLabel={t('toasts.update.later')}
      onDismiss={() => {
        setDismissed(update.targetVersion);
      }}
      title={t('toasts.update.title', { version: update.targetVersion })}
    />
  );
}
