import {
  canonicalTranslationLanguage,
  type MachineTranslationGateway,
} from '@/features/messages/domain/machine-translation';
import { err, ok } from '@/shared/result';

type TranslatorAvailability = 'available' | 'downloadable' | 'downloading' | 'unavailable';

type TranslatorOptions = {
  readonly sourceLanguage: string;
  readonly targetLanguage: string;
};

type BrowserTranslator = {
  destroy(): void;
  translate(input: string): Promise<string>;
};

export type BrowserTranslatorFactory = {
  availability(options: TranslatorOptions): Promise<TranslatorAvailability>;
  create(options: TranslatorOptions): Promise<BrowserTranslator>;
};

export class BrowserMachineTranslationGateway implements MachineTranslationGateway {
  readonly #factory: BrowserTranslatorFactory | null;

  constructor(factory: BrowserTranslatorFactory | null = readBrowserTranslatorFactory()) {
    this.#factory = factory;
  }

  async translate(request: Parameters<MachineTranslationGateway['translate']>[0]) {
    const sourceLanguage = canonicalTranslationLanguage(request.sourceLanguage);
    const targetLanguage = canonicalTranslationLanguage(request.targetLanguage);
    if (sourceLanguage === null || targetLanguage === null) {
      return err({ code: 'invalid_language' as const, retryable: false });
    }
    if (sourceLanguage === targetLanguage) {
      return err({ code: 'same_language' as const, retryable: false });
    }
    if (this.#factory === null) {
      return err({ code: 'unavailable' as const, retryable: false });
    }

    const options = { sourceLanguage, targetLanguage };
    let availability: TranslatorAvailability;
    try {
      availability = await this.#factory.availability(options);
    } catch {
      return err({ code: 'unavailable' as const, retryable: true });
    }
    if (availability === 'unavailable') {
      return err({ code: 'unavailable' as const, retryable: false });
    }

    let translator: BrowserTranslator;
    try {
      translator = await this.#factory.create(options);
    } catch {
      return err({ code: 'creation_failed' as const, retryable: true });
    }

    try {
      const translatedText = await translator.translate(request.originalText);
      return ok({
        originalText: request.originalText,
        provenance: 'machine' as const,
        sourceLanguage,
        targetLanguage,
        translatedText,
      });
    } catch {
      return err({ code: 'translation_failed' as const, retryable: true });
    } finally {
      translator.destroy();
    }
  }
}

function readBrowserTranslatorFactory(): BrowserTranslatorFactory | null {
  const candidate = (globalThis as unknown as { readonly Translator?: unknown }).Translator;
  if (candidate === null || (typeof candidate !== 'object' && typeof candidate !== 'function')) {
    return null;
  }
  const record = candidate as Readonly<Record<string, unknown>>;
  return typeof record.availability === 'function' && typeof record.create === 'function'
    ? (candidate as BrowserTranslatorFactory)
    : null;
}
