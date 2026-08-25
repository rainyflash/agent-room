import { Languages } from 'lucide-react';
import { useState } from 'react';
import { useTranslation } from 'react-i18next';

import {
  isLanguagePreference,
  type LanguagePreference,
} from '@/features/preferences/domain/account-preferences';
import { useOptionalAccountPreferences } from '@/features/preferences/ui/account-preferences-provider';
import { readLanguagePreference, setLanguagePreference } from '@/shared/i18n/i18n';

const preferences: readonly LanguagePreference[] = ['system', 'en', 'zh-CN'];

export function LanguageControl() {
  const { t } = useTranslation();
  const accountPreferences = useOptionalAccountPreferences();
  const [localPreference, setLocalPreference] = useState<LanguagePreference>(() =>
    readLanguagePreference(window.localStorage),
  );
  const preference = accountPreferences?.snapshot.values.language ?? localPreference;

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
          const next = event.target.value;
          if (!isLanguagePreference(next)) {
            throw new Error('语言选择器产生了不受支持的偏好值。');
          }
          if (accountPreferences === null) {
            setLocalPreference(next);
            void setLanguagePreference(next);
            return;
          }
          accountPreferences.setLanguage(next);
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
