import { Button } from '@agent-room/ui-system';
import { Languages, LoaderCircle, Sparkles } from 'lucide-react';
import { useState } from 'react';
import { useTranslation } from 'react-i18next';

import {
  canonicalTranslationLanguage,
  type MachineTranslation,
  type MachineTranslationFailure,
  type MachineTranslationGateway,
} from '@/features/messages/domain/machine-translation';
import { RestrictedMarkdown } from '@/features/messages/ui/restricted-markdown';

type TranslationState =
  | { readonly kind: 'idle' }
  | { readonly kind: 'running' }
  | { readonly failure: MachineTranslationFailure; readonly kind: 'failed' }
  | { readonly kind: 'ready'; readonly translation: MachineTranslation };

export type ExplicitMachineTranslationProps = {
  readonly gateway: MachineTranslationGateway;
  readonly mediaType: string;
  readonly originalText: string;
  readonly sourceLanguage: string | undefined;
};

export function ExplicitMachineTranslation({
  gateway,
  mediaType,
  originalText,
  sourceLanguage,
}: ExplicitMachineTranslationProps) {
  const { i18n, t } = useTranslation();
  const [state, setState] = useState<TranslationState>({ kind: 'idle' });
  const targetLanguage = i18n.resolvedLanguage ?? 'en';
  const source = sourceLanguage === undefined ? null : canonicalTranslationLanguage(sourceLanguage);
  const target = canonicalTranslationLanguage(targetLanguage);
  const sameLanguage = source !== null && target !== null && source === target;

  if (sameLanguage) {
    return null;
  }

  const translate = async (): Promise<void> => {
    if (sourceLanguage === undefined) {
      setState({
        failure: { code: 'invalid_language', retryable: false },
        kind: 'failed',
      });
      return;
    }
    setState({ kind: 'running' });
    const result = await gateway.translate({ originalText, sourceLanguage, targetLanguage });
    setState(
      result.ok
        ? { kind: 'ready', translation: result.value }
        : { failure: result.error, kind: 'failed' },
    );
  };

  return (
    <section className="content-inspector__translation">
      <header>
        <Languages aria-hidden="true" />
        <div>
          <strong>{t('messages.translation.title')}</strong>
          <p>{t('messages.translation.detail')}</p>
        </div>
      </header>
      {state.kind === 'ready' ? (
        <div className="content-inspector__translation-result">
          <span className="content-inspector__machine-badge">
            <Sparkles aria-hidden="true" />
            {t('messages.translation.machineBadge')}
          </span>
          {mediaType === 'text/markdown' ? (
            <RestrictedMarkdown source={state.translation.translatedText} />
          ) : (
            <pre>{state.translation.translatedText}</pre>
          )}
          <p>{t('messages.translation.originalPreserved')}</p>
        </div>
      ) : (
        <Button
          disabled={state.kind === 'running'}
          icon={
            state.kind === 'running' ? (
              <LoaderCircle aria-hidden="true" />
            ) : (
              <Languages aria-hidden="true" />
            )
          }
          onClick={() => void translate()}
          size="compact"
          tone="quiet"
        >
          {t(
            state.kind === 'running'
              ? 'messages.translation.running'
              : 'messages.translation.action',
          )}
        </Button>
      )}
      {state.kind === 'failed' ? (
        <p className="content-inspector__translation-failure" role="alert">
          {t(`messages.translation.failure.${state.failure.code}`)}
        </p>
      ) : null}
    </section>
  );
}
