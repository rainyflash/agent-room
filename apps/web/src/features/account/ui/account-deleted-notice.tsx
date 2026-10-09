import { Banner } from '@agent-room/ui-system';
import { useEffect, useState } from 'react';
import { useTranslation } from 'react-i18next';

import './account-deleted-notice.css';

// 删除账户后会退出登录、回到首页；首页凭这个记号说一声“正在删除”，只说一次。
const STORAGE_KEY = 'agent-room.account-deleted';
const NOTICE_WINDOW_MS = 60 * 60 * 1000;

export function rememberAccountDeletion(now: number = Date.now()): void {
  try {
    window.sessionStorage.setItem(STORAGE_KEY, String(now));
  } catch {
    // 存不了就不说，删除照样在进行。
  }
}

function readAccountDeletion(now: number): boolean {
  try {
    const stored = window.sessionStorage.getItem(STORAGE_KEY);
    if (stored === null) return false;
    const at = Number(stored);
    return Number.isFinite(at) && now - at >= 0 && now - at < NOTICE_WINDOW_MS;
  } catch {
    return false;
  }
}

function forgetAccountDeletion(): void {
  try {
    window.sessionStorage.removeItem(STORAGE_KEY);
  } catch {
    // 删不掉也没关系，一小时后不再显示。
  }
}

export function AccountDeletedNotice() {
  const { t } = useTranslation();
  const [shown] = useState(() => readAccountDeletion(Date.now()));
  useEffect(() => {
    if (shown) forgetAccountDeletion();
  }, [shown]);
  if (!shown) return null;
  return (
    <Banner className="account-deleted-notice" title={t('account.deleted.title')} tone="success">
      <p>{t('account.deleted.body')}</p>
    </Banner>
  );
}
