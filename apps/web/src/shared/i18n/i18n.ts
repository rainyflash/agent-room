import { createInstance } from 'i18next';
import { initReactI18next } from 'react-i18next';

import { resources, type SupportedLanguage } from './resources';
import type { LanguagePreference } from '@/features/preferences/domain/account-preferences';

const LANGUAGE_STORAGE_KEY = 'agent-room.language';

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
    return stored === 'en' || stored === 'zh-CN' ? stored : 'system';
  } catch {
    return 'system';
  }
}

export function effectiveLanguage(
  preference: LanguagePreference,
  systemLanguages: readonly string[],
): SupportedLanguage {
  return preference === 'system' ? resolveSystemLanguage(systemLanguages) : preference;
}

export const i18n = createInstance();

export async function initializeI18n(
  storage: Storage = window.localStorage,
  systemLanguages: readonly string[] = window.navigator.languages,
): Promise<void> {
  const preference = readLanguagePreference(storage);
  await i18n.use(initReactI18next).init({
    fallbackLng: 'en',
    interpolation: { escapeValue: false },
    lng: effectiveLanguage(preference, systemLanguages),
    resources,
    supportedLngs: ['en', 'zh-CN'],
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
  const language = effectiveLanguage(preference, systemLanguages);
  await i18n.changeLanguage(language);
  document.documentElement.lang = language;
}
