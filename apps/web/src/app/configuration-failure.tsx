import { AlertTriangle } from 'lucide-react';
import { useTranslation } from 'react-i18next';

import { LanguageControl } from '@/features/preferences/ui/language-control';

export type ConfigurationFailureProps = {
  readonly issues: readonly string[];
};

export function ConfigurationFailure({ issues }: ConfigurationFailureProps) {
  const { t } = useTranslation();
  return (
    <main className="boundary-page boundary-page--alert" id="main-content">
      <header className="boundary-page__header">
        <div className="brand-lockup brand-lockup--ink">
          <img alt="" src="/agent-room-mark.svg" />
          <span>{t('app.name')}</span>
        </div>
        <LanguageControl />
      </header>
      <section className="boundary-page__body">
        <AlertTriangle aria-hidden="true" className="boundary-page__icon" />
        <p className="eyebrow">RUNTIME / CONFIGURATION</p>
        <h1>{t('config.title')}</h1>
        <p>{t('config.description')}</p>
        <ul className="configuration-issues">
          {issues.map((issue) => (
            <li key={issue}>{issue}</li>
          ))}
        </ul>
      </section>
    </main>
  );
}
