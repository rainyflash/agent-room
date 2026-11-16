import { Banner, Button, Details, Spinner } from '@agent-room/ui-system';
import { useMutation, useQueryClient } from '@tanstack/react-query';
import { RefreshCw } from 'lucide-react';
import { useId, useState } from 'react';
import { useTranslation } from 'react-i18next';

import {
  roomLiveViewQueryKey,
  useRoomLiveView,
} from '@/features/private-rooms/data/room-live-view-queries';
import type { PrivateRoomFailure } from '@/features/private-rooms/domain/private-room';
import {
  isLiveViewCondition,
  type LiveViewCondition,
  type RoomLiveView,
  type RoomLiveViewGateway,
} from '@/features/private-rooms/domain/room-live-view';
import { PrivateRoomFailureNotice } from '@/features/private-rooms/ui/private-room-failure-notice';
import { formatDateTime } from '@/shared/i18n/formatters';
import type { Result } from '@/shared/result';

import './private-room-public-settings.css';

export type RoomLiveViewSettingsProps = {
  readonly catalogId: string;
  readonly gateway: RoomLiveViewGateway;
};

type LiveViewResult = Result<RoomLiveView, PrivateRoomFailure>;

/** 开关没成、服务器上的样子多半和这里看到的不一样了：重问一次，照新的说差哪一样、能不能开关。 */
const REFRESH_AFTER = new Set([
  'room_live.not_eligible',
  'room_live.forbidden',
  'room_live.conflict',
]);

/**
 * 房间设置“公开”一节里的“分享页放实况”一组（`specs/redesign/project-rooms.md`“网页”，3P-6b）。公开着的房间
 * 里谁都看得到开没开、此刻放没放、差哪一样；房主和管理员（服务器说 `canChange`）才有开关。打开先确认一次，
 * 说清楚不登录的人看得到什么；关掉不用确认。
 */
export function RoomLiveViewSettings({ catalogId, gateway }: RoomLiveViewSettingsProps) {
  const { t } = useTranslation();
  const headingId = useId();
  const state = useRoomLiveView(gateway, catalogId);

  let content;
  if (state.isPending) {
    content = (
      <p className="room-listing__note" role="status">
        <Spinner />
        {t('roomLive.loading')}
      </p>
    );
  } else if (state.isError || !state.data.ok) {
    content = (
      <>
        {state.data?.ok === false ? <RoomLiveViewFailureNotice failure={state.data.error} /> : null}
        <Button
          className="room-live-view__retry"
          icon={<RefreshCw aria-hidden="true" />}
          onClick={() => void state.refetch()}
          size="compact"
          tone="quiet"
        >
          {t('roomLive.retry')}
        </Button>
      </>
    );
  } else {
    content = <LiveViewControl catalogId={catalogId} gateway={gateway} view={state.data.value} />;
  }

  return (
    <section aria-labelledby={headingId} className="room-live-view">
      <h3 id={headingId}>{t('roomLive.title')}</h3>
      <p className="room-listing__note">{t('roomLive.detail')}</p>
      {content}
    </section>
  );
}

function LiveViewControl({
  catalogId,
  gateway,
  view,
}: RoomLiveViewSettingsProps & { readonly view: RoomLiveView }) {
  const { t, i18n } = useTranslation();
  const queryClient = useQueryClient();
  const labelId = useId();
  const stateId = useId();
  const [confirming, setConfirming] = useState(false);
  const update = useMutation({
    mutationFn: async (enabled: boolean) => await gateway.updateLiveView(catalogId, enabled),
    onSuccess: (result) => {
      setConfirming(false);
      if (result.ok) {
        // 顶栏那一行读的是同一份，跟着换。
        queryClient.setQueryData<LiveViewResult>(roomLiveViewQueryKey(catalogId), result);
        return;
      }
      if (REFRESH_AFTER.has(result.error.code)) {
        void queryClient.invalidateQueries({ queryKey: roomLiveViewQueryKey(catalogId) });
      }
    },
  });
  const failure = update.data?.ok === false ? update.data.error : null;
  // 关着、又有一样不满足时开不了（服务器会回 409）：开关灰着，下面说差哪一样。开着的照样能关。
  const blocked = !view.enabled && view.unmetConditions.length > 0;
  const state = view.live
    ? t('roomLive.state.live', {
        time: formatDateTime(view.enabledAtUnixMs ?? Date.now(), i18n.resolvedLanguage),
      })
    : t(view.enabled ? 'roomLive.state.paused' : 'roomLive.state.off');

  return (
    <div className="room-live-view__body">
      {view.canChange ? (
        <label className="room-listing__toggle">
          <input
            aria-describedby={stateId}
            aria-labelledby={labelId}
            // 确认之前照旧是关着：开关跟着服务器说的走，下面的确认框说打开以后会怎样。
            checked={view.enabled}
            disabled={update.isPending || blocked}
            onChange={(event) => {
              update.reset();
              if (event.target.checked) {
                setConfirming(true);
              } else {
                setConfirming(false);
                update.mutate(false);
              }
            }}
            role="switch"
            type="checkbox"
          />
          <span>
            <strong id={labelId}>{t('roomLive.switch')}</strong>
            <small aria-live="polite" id={stateId}>
              {update.isPending ? t('roomLive.saving') : state}
            </small>
          </span>
        </label>
      ) : (
        <p className="room-listing__state">{state}</p>
      )}
      {view.live || view.unmetConditions.length === 0 ? null : (
        <ul className="room-live-view__reasons">
          {view.unmetConditions.map((condition) => (
            <li key={condition}>{t(`roomLive.condition.${condition}`)}</li>
          ))}
        </ul>
      )}
      {confirming ? (
        <Banner
          action={
            <div className="room-listing__actions">
              <Button
                disabled={update.isPending}
                onClick={() => {
                  update.mutate(true);
                }}
                size="compact"
                tone="primary"
              >
                {update.isPending ? t('roomLive.saving') : t('roomLive.confirm.yes')}
              </Button>
              <Button
                disabled={update.isPending}
                onClick={() => {
                  setConfirming(false);
                }}
                size="compact"
                tone="ghost"
              >
                {t('roomLive.confirm.cancel')}
              </Button>
            </div>
          }
          title={t('roomLive.confirm.title')}
          tone="warning"
        >
          {t('roomLive.confirm.detail')}
        </Banner>
      ) : null}
      {view.canChange ? null : <p className="room-listing__note">{t('roomLive.managersOnly')}</p>}
      {failure === null ? null : <RoomLiveViewFailureNotice failure={failure} />}
    </div>
  );
}

/** 有自己说法的错误码：一句话说清楚，错误码和关联 ID 收进详情。别的照私人房间的通用说法。 */
const failureMessages = {
  'room_live.conflict': 'roomLive.error.conflict',
  'room_live.dependency_unavailable': 'roomLive.error.dependency_unavailable',
  'room_live.forbidden': 'roomLive.error.forbidden',
  'room_live.internal': 'roomLive.error.internal',
  'room_live.invalid': 'roomLive.error.invalid',
  'room_live.invalid_origin': 'roomLive.error.invalid_origin',
  'room_live.invalid_resource': 'roomLive.error.invalid',
  'room_live.not_eligible': 'roomLive.error.not_eligible',
  'room_live.not_found': 'roomLive.error.not_found',
} as const;

type LiveViewFailureMessage =
  | (typeof failureMessages)[keyof typeof failureMessages]
  | `roomLive.condition.${LiveViewCondition}`;

/** 开关实况没成时说的那一句；开不了（409）时说差的是哪一样。 */
export function liveViewFailureMessage(failure: PrivateRoomFailure): LiveViewFailureMessage | null {
  if (failure.code === 'room_live.not_eligible' && isLiveViewCondition(failure.condition)) {
    return `roomLive.condition.${failure.condition}`;
  }
  return Object.hasOwn(failureMessages, failure.code)
    ? failureMessages[failure.code as keyof typeof failureMessages]
    : null;
}

export function RoomLiveViewFailureNotice({ failure }: { readonly failure: PrivateRoomFailure }) {
  const { t } = useTranslation();
  const message = liveViewFailureMessage(failure);
  if (message === null) {
    return <PrivateRoomFailureNotice failure={failure} />;
  }
  return (
    <Banner tone="warning">
      <p>{t(message)}</p>
      <Details summary={t('roomLive.details')}>
        <code>{failure.code}</code>
        {failure.correlationId === undefined ? null : (
          <>
            {' · '}
            <code>{failure.correlationId}</code>
          </>
        )}
      </Details>
    </Banner>
  );
}
