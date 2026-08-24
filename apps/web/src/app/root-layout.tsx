import { Outlet } from '@tanstack/react-router';
import { useTranslation } from 'react-i18next';

import { UpdatePrompt } from '@/features/updates/ui/update-prompt';

export function RootLayout() {
  const { t } = useTranslation();
  return (
    <>
      <a className="skip-link" href="#main-content">
        {t('app.skipToContent')}
      </a>
      <Outlet />
      <UpdatePrompt />
    </>
  );
}
