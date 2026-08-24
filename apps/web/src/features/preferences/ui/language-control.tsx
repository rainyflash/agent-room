import { Languages } from 'lucide-react';
import { useState } from 'react';
import { useTranslation } from 'react-i18next';

import {
  readLanguagePreference,
  setLanguagePreference,
  type LanguagePreference,
} from '@/shared/i18n/i18n';

const preferences: readonly LanguagePreference[] = ['system', 'en', 'zh-CN'];

export function LanguageControl() {
  const { t } = useTranslation();
  const [preference, setPreference] = useState<LanguagePreference>(() =>
    readLanguagePreference(window.localStorage),
  );

  const labels: Readonly<Record<LanguagePreference, string>> = {
    en: t('app.language.english'),
    system: t('app.language.system'),
    'zh-CN': t('app.language.chinese'),
  };

  return (
    <label className="language-control">
      <span className="sr-only">{t('app.language')}</span>
      <Languages aria-hidden="true" />
      <select
        aria-label={t('app.language')}
        onChange={(event) => {
          const next = event.target.value as LanguagePreference;
          setPreference(next);
          void setLanguagePreference(next);
        }}
        value={preference}
      >
        {preferences.map((value) => (
          <option key={value} value={value}>
            {labels[value]}
          </option>
        ))}
      </select>
    </label>
  );
}
