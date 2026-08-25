import type { Result } from '@/shared/result';

export type MachineTranslationRequest = {
  readonly originalText: string;
  readonly sourceLanguage: string;
  readonly targetLanguage: string;
};

export type MachineTranslation = MachineTranslationRequest & {
  readonly provenance: 'machine';
  readonly translatedText: string;
};

export type MachineTranslationFailureCode =
  'invalid_language' | 'same_language' | 'unavailable' | 'creation_failed' | 'translation_failed';

export type MachineTranslationFailure = {
  readonly code: MachineTranslationFailureCode;
  readonly retryable: boolean;
};

export type MachineTranslationGateway = {
  translate(
    request: MachineTranslationRequest,
  ): Promise<Result<MachineTranslation, MachineTranslationFailure>>;
};

export function canonicalTranslationLanguage(language: string): string | null {
  try {
    const canonical = Intl.getCanonicalLocales(language.trim())[0];
    if (canonical === undefined) {
      return null;
    }
    if (canonical === 'zh-Hant' || canonical.startsWith('zh-Hant-')) {
      return 'zh-Hant';
    }
    const base = canonical.split('-')[0];
    return base ?? null;
  } catch {
    return null;
  }
}
