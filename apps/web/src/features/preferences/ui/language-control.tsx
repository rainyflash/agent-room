import { Languages } from 'lucide-react';
import { useState } from 'react';
import { useTranslation } from 'react-i18next';

import {
  isLanguagePreference,
  type LanguagePreference,
} from '@/features/preferences/domain/account-preferences';
import { useOptionalAccountPreferences } from '@/features/preferences/ui/account-preferences-provider';
import {
  readDeviceLanguageOverride,
  readLanguagePreference,
  setDeviceLanguageOverride,
  setLanguagePreference,
} from '@/shared/i18n/i18n';
import { isDeviceLanguageOverride, type DeviceLanguageOverride } from '@/shared/i18n/language';

const preferences: readonly LanguagePreference[] = ['system', 'en', 'zh-CN'];
type LanguageSelection = `account:${LanguagePreference}` | `device:${LanguagePreference}`;

export function LanguageControl() {
  const { t } = useTranslation();
  const accountPreferences = useOptionalAccountPreferences();
  const [localPreference, setLocalPreference] = useState<LanguagePreference>(() =>
    readLanguagePreference(window.localStorage),
  );
  const [localDeviceOverride, setLocalDeviceOverride] = useState<DeviceLanguageOverride>(() =>
    readDeviceLanguageOverride(window.localStorage),
  );
  const preference = accountPreferences?.snapshot.values.language ?? localPreference;
  const deviceOverride = accountPreferences?.deviceLanguageOverride ?? localDeviceOverride;
  const selection: LanguageSelection =
    deviceOverride === 'account' ? `account:${preference}` : `device:${deviceOverride}`;

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
          const next = parseLanguageSelection(event.target.value);
          if (next === null) {
            throw new Error('语言选择器产生了不受支持的偏好值。');
          }
          if (next.scope === 'device') {
            if (accountPreferences === null) {
              setLocalDeviceOverride(next.preference);
              void setDeviceLanguageOverride(next.preference, preference);
              return;
            }
            accountPreferences.setDeviceLanguageOverride(next.preference);
            return;
          }
          if (accountPreferences === null) {
            setLocalPreference(next.preference);
            setLocalDeviceOverride('account');
            void setDeviceLanguageOverride('account', next.preference);
            void setLanguagePreference(next.preference);
            return;
          }
          accountPreferences.setDeviceLanguageOverride('account');
          accountPreferences.setLanguage(next.preference);
        }}
        value={selection}
      >
        <optgroup label={t('app.language.account')}>
          {preferences.map((value) => (
            <option key={`account:${value}`} value={`account:${value}`}>
              {labels[value]}
            </option>
          ))}
        </optgroup>
        <optgroup label={t('app.language.device')}>
          {preferences.map((value) => (
            <option key={`device:${value}`} value={`device:${value}`}>
              {labels[value]}
            </option>
          ))}
        </optgroup>
      </select>
    </label>
  );
}

function parseLanguageSelection(
  value: string,
): { readonly preference: LanguagePreference; readonly scope: 'account' | 'device' } | null {
  const [scope, preference, extra] = value.split(':');
  if (
    extra !== undefined ||
    (scope !== 'account' && scope !== 'device') ||
    preference === undefined ||
    !isLanguagePreference(preference)
  ) {
    return null;
  }
  if (scope === 'device' && !isDeviceLanguageOverride(preference)) {
    return null;
  }
  return { preference, scope };
}
