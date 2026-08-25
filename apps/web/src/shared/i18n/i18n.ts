import { createInstance } from 'i18next';
import { initReactI18next } from 'react-i18next';

import {
  isDeviceLanguageOverride,
  isLanguagePreference,
  selectLanguagePreference,
  supportedLanguages,
  type DeviceLanguageOverride,
  type LanguagePreference,
  type SupportedLanguage,
} from './language';
import { resources } from './resources';

const LANGUAGE_STORAGE_KEY = 'agent-room.language';
const DEVICE_LANGUAGE_OVERRIDE_STORAGE_KEY = 'agent-room.language.device-override';

const languageAliases: Readonly<Record<string, SupportedLanguage>> = {
  en: 'en',
  'en-US': 'en',
  zh: 'zh-CN',
  'zh-CN': 'zh-CN',
  'zh-Hans': 'zh-CN',
};

export function resolveSystemLanguage(languages: readonly string[]): SupportedLanguage {
  for (const language of languages) {
    const exact = languageAliases[language];
    if (exact !== undefined) {
      return exact;
    }
    const base = language.split('-')[0];
    if (base !== undefined) {
      const matched = languageAliases[base];
      if (matched !== undefined) {
        return matched;
      }
    }
  }
  return 'en';
}

export function readLanguagePreference(storage: Storage): LanguagePreference {
  try {
    const stored = storage.getItem(LANGUAGE_STORAGE_KEY);
    return stored !== null && isLanguagePreference(stored) ? stored : 'system';
  } catch {
    return 'system';
  }
}

export function readDeviceLanguageOverride(storage: Storage): DeviceLanguageOverride {
  try {
    const stored = storage.getItem(DEVICE_LANGUAGE_OVERRIDE_STORAGE_KEY);
    return stored !== null && isDeviceLanguageOverride(stored) ? stored : 'account';
  } catch {
    return 'account';
  }
}

export function effectiveLanguage(
  accountPreference: LanguagePreference,
  systemLanguages: readonly string[],
  deviceOverride: DeviceLanguageOverride = 'account',
): SupportedLanguage {
  const preference = selectLanguagePreference(accountPreference, deviceOverride);
  return preference === 'system' ? resolveSystemLanguage(systemLanguages) : preference;
}

export const i18n = createInstance();

export async function initializeI18n(
  storage: Storage = window.localStorage,
  systemLanguages: readonly string[] = window.navigator.languages,
): Promise<void> {
  const accountPreference = readLanguagePreference(storage);
  const deviceOverride = readDeviceLanguageOverride(storage);
  await i18n.use(initReactI18next).init({
    fallbackLng: 'en',
    interpolation: { escapeValue: false },
    lng: effectiveLanguage(accountPreference, systemLanguages, deviceOverride),
    resources,
    supportedLngs: [...supportedLanguages],
  });
  document.documentElement.lang = i18n.resolvedLanguage ?? 'en';
}

export async function setLanguagePreference(
  preference: LanguagePreference,
  storage: Storage = window.localStorage,
  systemLanguages: readonly string[] = window.navigator.languages,
): Promise<void> {
  try {
    if (preference === 'system') {
      storage.removeItem(LANGUAGE_STORAGE_KEY);
    } else {
      storage.setItem(LANGUAGE_STORAGE_KEY, preference);
    }
  } catch {
    // 存储不可用时仍允许本次会话切换语言。
  }
  const language = effectiveLanguage(
    preference,
    systemLanguages,
    readDeviceLanguageOverride(storage),
  );
  await applyLanguage(language);
}

export async function setDeviceLanguageOverride(
  override: DeviceLanguageOverride,
  accountPreference: LanguagePreference,
  storage: Storage = window.localStorage,
  systemLanguages: readonly string[] = window.navigator.languages,
): Promise<void> {
  try {
    if (override === 'account') {
      storage.removeItem(DEVICE_LANGUAGE_OVERRIDE_STORAGE_KEY);
    } else {
      storage.setItem(DEVICE_LANGUAGE_OVERRIDE_STORAGE_KEY, override);
    }
  } catch {
    // 临时覆盖在存储被禁用时仍对当前运行会话生效。
  }
  await applyLanguage(effectiveLanguage(accountPreference, systemLanguages, override));
}

async function applyLanguage(language: SupportedLanguage): Promise<void> {
  await i18n.changeLanguage(language);
  document.documentElement.lang = language;
}
