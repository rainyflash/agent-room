import { Banner, Button, CopyBlock, Spinner } from '@agent-room/ui-system';
import { useMutation, useQueryClient } from '@tanstack/react-query';
import { RefreshCw } from 'lucide-react';
import { useState, type ReactNode } from 'react';
import { useTranslation } from 'react-i18next';

import {
  privateRoomListQueryKey,
  privateRoomListingQueryKey,
  usePrivateRoomListing,
} from '@/features/private-rooms/data/private-room-queries';
import type { PrivateRoom, PrivateRoomFailure } from '@/features/private-rooms/domain/private-room';
import {
  grantableToNewMembers,
  sameSettings,
  settingsOf,
  shareable,
  type PrivateRoomListing,
  type PrivateRoomListingGateway,
  type PrivateRoomListingSettings,
} from '@/features/private-rooms/domain/private-room-listing';
import type { ProjectRoomGateway } from '@/features/private-rooms/domain/project-rooms';
import type { RoomLiveViewGateway } from '@/features/private-rooms/domain/room-live-view';
import { useRoomRepository } from '@/features/private-rooms/data/project-room-queries';
import { PrivateRoomCapabilityEditor } from '@/features/private-rooms/ui/private-room-capability-editor';
import { PrivateRoomFailureNotice } from '@/features/private-rooms/ui/private-room-failure-notice';
import { ProjectRoomRepository } from '@/features/private-rooms/ui/project-room-repository';
import { RoomListingChoices } from '@/features/private-rooms/ui/room-listing-choices';
import { RoomLiveViewSettings } from '@/features/private-rooms/ui/room-live-view-settings';
import type { Result } from '@/shared/result';

import './private-room-public-settings.css';

export type PrivateRoomPublicSettingsProps = {
  readonly listings: PrivateRoomListingGateway;
  /** 分享页放实况（specs/redesign/project-rooms.md）；没有这项服务时不出“分享页放实况”一组。 */
  readonly liveViews?: RoomLiveViewGateway | undefined;
  readonly principalId: string;
  /** 绑 GitHub 仓库（specs/redesign/project-rooms.md）；没有这项服务时不出“GitHub 仓库”一组。 */
  readonly projects?: ProjectRoomGateway | undefined;
  readonly room: PrivateRoom;
};

type ListingResult = Result<PrivateRoomListing, PrivateRoomFailure>;

/**
 * 房间设置的“公开”一节（`specs/public-rooms/design.md`“网页”）：公开还是私密、别人怎么进、新成员
 * 拿到什么、没有账号的网络 Agent 进不进得来，以及分享链接。公开着的房间多“分享页放实况”一组（谁都看得到，
 * 房主和管理员能开关），房主还看得到“GitHub 仓库”一组（`specs/redesign/project-rooms.md`）。公开设置房主能改，
 * 别的成员只看得到。公开和私密之间切换先在这一节里确认一次，说清楚加密怎么变、不登录的人看得到哪些。
 */
export function PrivateRoomPublicSettings({
  listings,
  liveViews,
  principalId,
  projects,
  room,
}: PrivateRoomPublicSettingsProps) {
  const { t } = useTranslation();
  const listing = usePrivateRoomListing(listings, room.catalogId);
  if (listing.isPending) {
    return (
      <p className="room-listing__note" role="status">
        <Spinner />
        {t('privateRooms.listing.loading')}
      </p>
    );
  }
  if (listing.isError || !listing.data.ok) {
    return (
      <div className="room-listing">
        {listing.data?.ok === false ? (
          <PrivateRoomFailureNotice failure={listing.data.error} />
        ) : null}
        <Button
          icon={<RefreshCw aria-hidden="true" />}
          onClick={() => void listing.refetch()}
          size="compact"
          tone="quiet"
        >
          {t('privateRooms.listing.retry')}
        </Button>
      </div>
    );
  }
  const current = listing.data.value;
  // 公开着的房间里谁都看得到实况开没开；房主和管理员能开关（服务器说的 `canChange`）。
  const liveView =
    current.listed && liveViews !== undefined ? (
      <RoomLiveViewSettings catalogId={room.catalogId} gateway={liveViews} />
    ) : null;
  return (
    <div className="room-listing">
      {current.platformHold === null ? null : (
        <Banner role={null} title={t('privateRooms.listing.held.title')} tone="warning">
          {t('privateRooms.listing.held.detail', { reason: current.platformHold.reason })}
        </Banner>
      )}
      {room.ownerPrincipalId === principalId ? (
        <OwnerSettings
          current={current}
          listings={listings}
          liveView={liveView}
          liveViewAvailable={liveViews !== undefined}
          principalId={principalId}
          projects={projects}
          room={room}
        />
      ) : (
        <>
          <ListingSummary current={current} />
          {liveView}
        </>
      )}
    </div>
  );
}

function OwnerSettings({
  current,
  listings,
  liveView,
  liveViewAvailable,
  principalId,
  projects,
  room,
}: {
  readonly current: PrivateRoomListing;
  readonly listings: PrivateRoomListingGateway;
  /** “分享页放实况”一组，排在分享链接后面；房间不公开时是 `null`。 */
  readonly liveView: ReactNode;
  /** 有没有实况这项服务：设成公开的确认框里才说实况能在这一节打开。 */
  readonly liveViewAvailable: boolean;
  readonly principalId: string;
  readonly projects: ProjectRoomGateway | undefined;
  readonly room: PrivateRoom;
}) {
  const { t } = useTranslation();
  const queryClient = useQueryClient();
  // 绑着仓库的房间不能换分享码：链接写在仓库里。和“GitHub 仓库”一组读同一份。
  const repository = useRoomRepository(projects, room.catalogId, current.listed);
  const repositoryBound = repository.data?.ok === true && repository.data.value !== null;
  // 改了才记下；没改时跟着服务器上的。
  const [draft, setDraft] = useState<PrivateRoomListingSettings | null>(null);
  const [confirming, setConfirming] = useState(false);
  const saved = settingsOf(current);
  const shown = draft ?? saved;
  const changed = !sameSettings(shown, saved);
  const switching = shown.listed !== saved.listed;

  const update = useMutation({
    mutationFn: async (settings: PrivateRoomListingSettings) =>
      await listings.updateListing(room.catalogId, settings),
    onSuccess: async (result) => {
      setConfirming(false);
      if (!result.ok) return;
      queryClient.setQueryData<ListingResult>(privateRoomListingQueryKey(room.catalogId), result);
      setDraft(null);
      // 改回私密会打开加密：房间列表和房间页跟着重读。实况（`roomLiveViewQueryKey`）也挂在这个键下面：
      // 改回私密、换了加入方式时服务器已经把它关了，设置里的开关和顶栏那一行一起重问。
      await queryClient.invalidateQueries({
        queryKey: privateRoomListQueryKey,
        refetchType: 'active',
      });
    },
  });
  const change = (next: Partial<PrivateRoomListingSettings>): void => {
    update.reset();
    setConfirming(false);
    setDraft({ ...shown, ...next });
  };
  const failure = update.data?.ok === false ? update.data.error : null;

  return (
    <>
      <RoomListingChoices
        disabled={update.isPending}
        name={room.catalogId}
        onChange={change}
        value={shown}
      />
      {shown.listed ? (
        <>
          <PrivateRoomCapabilityEditor
            disabled={update.isPending}
            grantable={grantableToNewMembers}
            legend={t('privateRooms.listing.memberPermissions')}
            onChange={(memberPermissions) => {
              change({ memberPermissions });
            }}
            value={shown.memberPermissions}
          />
          <label className="room-listing__toggle">
            <input
              checked={shown.allowNetworkAgents}
              disabled={update.isPending}
              onChange={(event) => {
                change({ allowNetworkAgents: event.target.checked });
              }}
              type="checkbox"
            />
            <span>
              <strong>{t('privateRooms.listing.networkAgents.label')}</strong>
              <small>{t('privateRooms.listing.networkAgents.detail')}</small>
            </span>
          </label>
          {confirming ? null : (
            <p className="room-listing__note">{t('privateRooms.listing.readers')}</p>
          )}
        </>
      ) : null}
      {/* 确认的时候确认框里会说一遍谁看得到、加密怎么变，这里就不重复了。 */}
      {confirming ? null : <EncryptionNote current={current} listed={shown.listed} />}
      {confirming ? (
        <SwitchConfirmation
          busy={update.isPending}
          current={current}
          // 以前就有的房间设成公开时实况不会自己打开（新建时才默认开）：能开的才说一句在这一节里开。
          liveViewLater={
            liveViewAvailable && !current.endToEndEncrypted && shown.joinMode === 'open'
          }
          listed={shown.listed}
          onCancel={() => {
            setConfirming(false);
          }}
          onConfirm={() => {
            update.mutate(shown);
          }}
        />
      ) : (
        <div className="room-listing__actions">
          <Button
            disabled={!changed || update.isPending}
            onClick={() => {
              if (switching) {
                setConfirming(true);
              } else {
                update.mutate(shown);
              }
            }}
            tone="primary"
          >
            {update.isPending ? t('privateRooms.listing.saving') : t('privateRooms.listing.save')}
          </Button>
          {changed ? (
            <Button
              disabled={update.isPending}
              onClick={() => {
                update.reset();
                setDraft(null);
              }}
              tone="ghost"
            >
              {t('privateRooms.listing.discard')}
            </Button>
          ) : null}
        </div>
      )}
      {update.data?.ok === true ? (
        <p className="room-listing__note" role="status">
          {t('privateRooms.listing.saved')}
        </p>
      ) : null}
      {failure === null ? null : failure.code === 'room_listing.limit_reached' ? (
        <Banner tone="warning">{t('privateRooms.listing.limitReached')}</Banner>
      ) : (
        <PrivateRoomFailureNotice failure={failure} />
      )}
      {saved.listed ? (
        <ShareLink
          current={current}
          owner={{ catalogId: room.catalogId, listings, repositoryBound }}
        />
      ) : null}
      {liveView}
      {saved.listed && projects !== undefined ? (
        <ProjectRoomRepository
          catalogId={room.catalogId}
          gateway={projects}
          principalId={principalId}
        />
      ) : null}
    </>
  );
}

/** 公开着的房间加没加密、切换以后会怎样。 */
function EncryptionNote({
  current,
  listed,
}: {
  readonly current: PrivateRoomListing;
  readonly listed: boolean;
}) {
  const { t } = useTranslation();
  if (current.endToEndEncrypted) {
    return listed ? (
      <p className="room-listing__note">{t('privateRooms.listing.encrypted')}</p>
    ) : null;
  }
  return (
    <p className="room-listing__note room-listing__note--plain">
      {t(listed ? 'privateRooms.listing.plain' : 'privateRooms.listing.plainBecomingPrivate')}
    </p>
  );
}

function SwitchConfirmation({
  busy,
  current,
  liveViewLater,
  listed,
  onCancel,
  onConfirm,
}: {
  readonly busy: boolean;
  readonly current: PrivateRoomListing;
  /** 设成公开以后能在这一节里打开实况：确认框里说一句它不会自己打开。 */
  readonly liveViewLater: boolean;
  readonly listed: boolean;
  readonly onCancel: () => void;
  readonly onConfirm: () => void;
}) {
  const { t } = useTranslation();
  const action = listed ? 'public' : 'private';
  return (
    <Banner
      action={
        <div className="room-listing__actions">
          <Button disabled={busy} onClick={onConfirm} size="compact" tone="primary">
            {busy
              ? t('privateRooms.listing.saving')
              : t(`privateRooms.listing.confirm.${action}.yes`)}
          </Button>
          <Button disabled={busy} onClick={onCancel} size="compact" tone="ghost">
            {t('privateRooms.listing.confirm.cancel')}
          </Button>
        </div>
      }
      title={t(`privateRooms.listing.confirm.${action}.title`)}
      tone="warning"
    >
      <span className="room-listing__confirm">
        {listed ? (
          <>
            <span>{t('privateRooms.listing.readers')}</span>
            <span>
              {t(
                current.endToEndEncrypted
                  ? 'privateRooms.listing.confirm.public.encrypted'
                  : 'privateRooms.listing.confirm.public.plain',
              )}
            </span>
            {liveViewLater ? (
              <span>{t('privateRooms.listing.confirm.public.liveView')}</span>
            ) : null}
          </>
        ) : (
          <>
            <span>{t('privateRooms.listing.confirm.private.detail')}</span>
            {current.endToEndEncrypted ? null : (
              <span>{t('privateRooms.listing.confirm.private.encryption')}</span>
            )}
          </>
        )}
      </span>
    </Banner>
  );
}

/** 不是房主：只看得到现在怎样，和分享链接。 */
function ListingSummary({ current }: { readonly current: PrivateRoomListing }) {
  const { t } = useTranslation();
  return (
    <>
      <p className="room-listing__state">
        {current.listed
          ? t('privateRooms.listing.summary.public', {
              joinMode: t(`privateRooms.listing.joinMode.${current.joinMode}.label`),
            })
          : t('privateRooms.listing.summary.private')}
      </p>
      {current.listed && !current.endToEndEncrypted ? (
        <p className="room-listing__note room-listing__note--plain">
          {t('privateRooms.listing.plain')}
        </p>
      ) : null}
      {current.listed ? <ShareLink current={current} /> : null}
      <p className="room-listing__note">{t('privateRooms.listing.ownerOnly')}</p>
    </>
  );
}

/** 分享链接；给了 `owner` 才有“换一个分享码”，绑着仓库时没有（链接写在仓库里）。 */
function ShareLink({
  current,
  owner,
}: {
  readonly current: PrivateRoomListing;
  readonly owner?: {
    readonly catalogId: string;
    readonly listings: PrivateRoomListingGateway;
    readonly repositoryBound: boolean;
  };
}) {
  const { t } = useTranslation();
  const queryClient = useQueryClient();
  const [confirming, setConfirming] = useState(false);
  const replace = useMutation({
    mutationFn: async () =>
      owner === undefined ? null : await owner.listings.changeShareCode(owner.catalogId),
    onSuccess: (result) => {
      setConfirming(false);
      if (result?.ok !== true || owner === undefined) return;
      queryClient.setQueryData<ListingResult>(privateRoomListingQueryKey(owner.catalogId), result);
    },
  });
  if (!shareable(current)) {
    return current.platformHold === null && current.joinMode === 'invite_only' ? (
      <p className="room-listing__note">{t('privateRooms.listing.share.inviteOnly')}</p>
    ) : null;
  }
  const failure = replace.data?.ok === false ? replace.data.error : null;
  return (
    <section className="room-listing__share">
      <h3>{t('privateRooms.listing.share.title')}</h3>
      <p className="room-listing__note">{t('privateRooms.listing.share.detail')}</p>
      <CopyBlock
        copiedLabel={t('privateRooms.listing.share.copied')}
        copyLabel={t('privateRooms.listing.share.copy')}
        failedLabel={t('privateRooms.listing.share.copyFailed')}
        text={current.shareUrl}
        textLabel={t('privateRooms.listing.share.label')}
      />
      {replace.data?.ok === true ? (
        <p className="room-listing__note" role="status">
          {t('privateRooms.listing.share.replaced')}
        </p>
      ) : null}
      {owner?.repositoryBound === true || failure?.code === 'room_listing.repository_bound' ? (
        <p className="room-listing__note">{t('projectRooms.shareCodeBound')}</p>
      ) : owner === undefined ? null : confirming ? (
        <Banner
          action={
            <div className="room-listing__actions">
              <Button
                disabled={replace.isPending}
                onClick={() => {
                  replace.mutate();
                }}
                size="compact"
                tone="alert"
              >
                {t('privateRooms.listing.share.replaceYes')}
              </Button>
              <Button
                disabled={replace.isPending}
                onClick={() => {
                  setConfirming(false);
                }}
                size="compact"
                tone="ghost"
              >
                {t('privateRooms.listing.confirm.cancel')}
              </Button>
            </div>
          }
          tone="warning"
        >
          {t('privateRooms.listing.share.replaceWarning')}
        </Banner>
      ) : (
        <Button
          icon={<RefreshCw aria-hidden="true" />}
          onClick={() => {
            replace.reset();
            setConfirming(true);
          }}
          size="compact"
          tone="ghost"
        >
          {t('privateRooms.listing.share.replace')}
        </Button>
      )}
      {failure === null || failure.code === 'room_listing.repository_bound' ? null : (
        <PrivateRoomFailureNotice failure={failure} />
      )}
    </section>
  );
}
