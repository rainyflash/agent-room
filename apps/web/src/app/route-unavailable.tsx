import { ArrowLeft, Compass } from 'lucide-react';
import { useTranslation } from 'react-i18next';

import { EntryCard, EntryShell } from '@/shared/ui/entry-shell';

export type RouteUnavailableProps = {
  readonly invalid?: boolean;
  readonly routeLabel: string;
};

/** 链接不对，或者页面还没做：说一句人话，给“回到房间”，地址收进详情。 */
export function RouteUnavailable({ invalid = false, routeLabel }: RouteUnavailableProps) {
  const { t } = useTranslation();
  return (
    <EntryShell>
      <EntryCard
        actions={
          <a className="ar-button ar-button--large ar-button--primary" href="/rooms">
            <span className="ar-button__icon">
              <ArrowLeft aria-hidden="true" />
            </span>
            <span>{t('entry.backToRooms')}</span>
          </a>
        }
        detail={invalid ? t('route.invalid.description') : t('app.notImplemented.description')}
        details={
          <dl>
            <div>
              <dt>{t('entry.address')}</dt>
              <dd>
                <code>{routeLabel}</code>
              </dd>
            </div>
          </dl>
        }
        icon={<Compass />}
        title={invalid ? t('route.invalid.title') : t('app.notImplemented.title')}
      />
    </EntryShell>
  );
}
