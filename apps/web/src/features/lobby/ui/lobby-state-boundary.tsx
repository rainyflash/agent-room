import { Button } from '@agent-room/ui-system';
import { LoaderCircle, RadioTower, RotateCw } from 'lucide-react';
import { useTranslation } from 'react-i18next';

import type { LobbyRoomState } from '@/features/lobby/application/lobby-room-store';
import type { LobbyFailureCode } from '@/features/lobby/domain/lobby';
import { LanguageControl } from '@/features/preferences/ui/language-control';

type LobbyPendingState = Exclude<LobbyRoomState, { readonly kind: 'ready' }>;

export type LobbyStateBoundaryProps = {
  readonly onRetry: () => void;
  readonly state: LobbyPendingState;
};

const failureKeyByCode: Readonly<Record<LobbyFailureCode, string>> = Object.freeze({
  'lobby.matrix_unavailable': 'matrixUnavailable',
  'lobby.room_not_joined': 'roomNotJoined',
  'lobby.room_projection_invalid': 'projectionInvalid',
});

export function LobbyStateBoundary({ onRetry, state }: LobbyStateBoundaryProps) {
  const { t } = useTranslation();
  const loading = state.kind === 'loading';
  const failureKey = state.kind === 'failed' ? failureKeyByCode[state.code] : null;
  const title = loading
    ? t('lobby.loading.title')
    : t(`lobby.failure.${failureKey ?? 'projectionInvalid'}.title`);
  const detail = loading
    ? t('lobby.loading.detail')
    : t(`lobby.failure.${failureKey ?? 'projectionInvalid'}.detail`);

  return (
    <main className="lobby-boundary" id="main-content">
      <header className="lobby-boundary__topbar">
        <a aria-label={t('app.name')} className="brand-lockup brand-lockup--ink" href="/connect">
          <img alt="" src="/agent-room-mark.svg" />
          <span>{t('app.name')}</span>
        </a>
        <LanguageControl />
      </header>
      <section aria-live="polite" className="lobby-boundary__body">
        {loading ? (
          <LoaderCircle aria-hidden="true" className="lobby-boundary__icon lobby-boundary__spin" />
        ) : (
          <RadioTower aria-hidden="true" className="lobby-boundary__icon" />
        )}
        <p className="eyebrow">{t('lobby.boundary.eyebrow')}</p>
        <h1>{title}</h1>
        <p>{detail}</p>
        {state.kind === 'failed' ? (
          <div className="lobby-boundary__actions">
            {state.retryable ? (
              <Button icon={<RotateCw aria-hidden="true" />} onClick={onRetry} tone="primary">
                {t('lobby.failure.retry')}
              </Button>
            ) : null}
            <a className="lobby-boundary__link" href="/connect">
              {t('lobby.failure.connect')}
            </a>
          </div>
        ) : null}
      </section>
    </main>
  );
}
