import { Button, Spinner } from '@agent-room/ui-system';
import { RotateCw, TriangleAlert } from 'lucide-react';
import { useTranslation } from 'react-i18next';

import type { LobbyRoomState } from '@/features/lobby/application/lobby-room-store';
import type { LobbyFailureCode } from '@/features/lobby/domain/lobby';
import type { TranslationKey } from '@/shared/i18n/resources';
import { EntryCard, EntryShell } from '@/shared/ui/entry-shell';

type LobbyPendingState = Exclude<LobbyRoomState, { readonly kind: 'ready' }>;

export type LobbyStateBoundaryProps = {
  readonly onRetry: () => void;
  readonly state: LobbyPendingState;
};

const failureCopyByCode: Readonly<
  Record<LobbyFailureCode, { readonly detail: TranslationKey; readonly title: TranslationKey }>
> = Object.freeze({
  'lobby.matrix_unavailable': {
    detail: 'lobby.failure.matrixUnavailable.detail',
    title: 'lobby.failure.matrixUnavailable.title',
  },
  'lobby.room_not_joined': {
    detail: 'lobby.failure.roomNotJoined.detail',
    title: 'lobby.failure.roomNotJoined.title',
  },
  'lobby.room_projection_invalid': {
    detail: 'lobby.failure.projectionInvalid.detail',
    title: 'lobby.failure.projectionInvalid.title',
  },
});

/** 房间还在打开，或者打不开：一张卡片说清楚，出错时给“重试”和“回到房间”，错误码收进详情。 */
export function LobbyStateBoundary({ onRetry, state }: LobbyStateBoundaryProps) {
  const { t } = useTranslation();
  if (state.kind === 'loading') {
    return (
      <EntryShell>
        <EntryCard
          detail={t('lobby.loading.detail')}
          icon={<Spinner />}
          title={t('lobby.loading.title')}
        />
      </EntryShell>
    );
  }
  const copy = failureCopyByCode[state.code];
  return (
    <EntryShell>
      <EntryCard
        actions={
          <>
            {state.retryable ? (
              <Button
                icon={<RotateCw aria-hidden="true" />}
                onClick={onRetry}
                size="large"
                tone="primary"
              >
                {t('lobby.failure.retry')}
              </Button>
            ) : null}
            <a className="ar-button ar-button--large ar-button--ghost" href="/rooms">
              {t('entry.backToRooms')}
            </a>
          </>
        }
        detail={t(copy.detail)}
        details={
          <dl>
            <div>
              <dt>{t('entry.errorCode')}</dt>
              <dd>
                <code>{state.code}</code>
              </dd>
            </div>
          </dl>
        }
        icon={<TriangleAlert />}
        title={t(copy.title)}
        tone="alert"
      />
    </EntryShell>
  );
}
