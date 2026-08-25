export const supportedLanguages = ['en', 'zh-CN'] as const;
export const languagePreferences = ['system', ...supportedLanguages] as const;
export const deviceLanguageOverrides = ['account', ...languagePreferences] as const;

export type SupportedLanguage = (typeof supportedLanguages)[number];
export type LanguagePreference = (typeof languagePreferences)[number];
export type DeviceLanguageOverride = (typeof deviceLanguageOverrides)[number];

const languagePreferenceSet: ReadonlySet<string> = new Set(languagePreferences);
const deviceLanguageOverrideSet: ReadonlySet<string> = new Set(deviceLanguageOverrides);

export function isLanguagePreference(value: string): value is LanguagePreference {
  return languagePreferenceSet.has(value);
}

export function isDeviceLanguageOverride(value: string): value is DeviceLanguageOverride {
  return deviceLanguageOverrideSet.has(value);
}

export function selectLanguagePreference(
  accountPreference: LanguagePreference,
  deviceOverride: DeviceLanguageOverride,
): LanguagePreference {
  return deviceOverride === 'account' ? accountPreference : deviceOverride;
}
