import { Button } from '@agent-room/ui-system';
import { ArrowLeft, Construction } from 'lucide-react';
import { useTranslation } from 'react-i18next';

import { LanguageControl } from '@/features/preferences/ui/language-control';

export type RouteUnavailableProps = {
  readonly invalid?: boolean;
  readonly routeLabel: string;
};

export function RouteUnavailable({ invalid = false, routeLabel }: RouteUnavailableProps) {
  const { t } = useTranslation();
  return (
    <main className="boundary-page" id="main-content">
      <header className="boundary-page__header">
        <a className="brand-lockup brand-lockup--ink" href="/connect">
          <img alt="" src="/agent-room-mark.svg" />
          <span>{t('app.name')}</span>
        </a>
        <LanguageControl />
      </header>
      <section className="boundary-page__body">
        <Construction aria-hidden="true" className="boundary-page__icon" />
        <p className="eyebrow">
          {invalid ? t('route.invalid.eyebrow') : t('app.notImplemented.eyebrow')}
        </p>
        <h1>{invalid ? t('route.invalid.title') : t('app.notImplemented.title')}</h1>
        <p className="boundary-page__route">{routeLabel}</p>
        <p>{invalid ? t('route.invalid.description') : t('app.notImplemented.description')}</p>
        <Button
          icon={<ArrowLeft aria-hidden="true" />}
          onClick={() => {
            window.location.assign('/connect');
          }}
          size="large"
          tone="primary"
        >
          {t('app.notImplemented.back')}
        </Button>
      </section>
    </main>
  );
}
