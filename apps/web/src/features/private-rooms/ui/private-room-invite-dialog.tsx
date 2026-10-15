import { Banner, Button, CopyBlock, Details, Dialog, Spinner } from '@agent-room/ui-system';
import { useMutation, useQueryClient } from '@tanstack/react-query';
import { Link2, Link2Off, RefreshCw, UserPlus } from 'lucide-react';
import { useState } from 'react';
import { useTranslation } from 'react-i18next';

import {
  privateRoomInviteLinkQueryKey,
  usePrivateRoomInviteLink,
} from '@/features/private-rooms/data/private-room-queries';
import {
  grantableBy,
  intersectPermissions,
  privateRoomCapabilities,
  type PrivateRoom,
  type PrivateRoomFailure,
  type PrivateRoomGateway,
  type PrivateRoomPermissions,
} from '@/features/private-rooms/domain/private-room';
import {
  DEFAULT_INVITE_PERMISSIONS,
  type PrivateRoomInviteGateway,
  type PrivateRoomInviteLink,
} from '@/features/private-rooms/domain/private-room-invite-link';
import { PrivateRoomAccountInvite } from '@/features/private-rooms/ui/private-room-account-invite';
import { PrivateRoomCapabilityEditor } from '@/features/private-rooms/ui/private-room-capability-editor';
import { PrivateRoomFailureNotice } from '@/features/private-rooms/ui/private-room-failure-notice';
import { formatDateTime } from '@/shared/i18n/formatters';
import { err, type Result } from '@/shared/result';

export type PrivateRoomInviteDialogProps = {
  readonly invites: PrivateRoomInviteGateway;
  readonly onClose: () => void;
  readonly principalId: string;
  readonly room: PrivateRoom;
  readonly rooms: PrivateRoomGateway;
};

type LinkResult = Result<PrivateRoomInviteLink, PrivateRoomFailure>;

/** 查链接时这两种回答不算出错：还没有链接，或者有却看不到（这台服务器没存能再看的那份）。 */
const expectedLinkFailures = new Set(['invite_link.not_found', 'invite_link.not_revealable']);
const forbiddenCodes = new Set(['invite_link.forbidden', 'private_room.forbidden']);

/**
 * 房间页“接入 Agent”旁边的“邀请人”（`specs/room-people/design.md`）：一条链接，复制了发给朋友，
 * 他们登录后就能进来；能换一条、能停用。链接给的权限收在“设置”里，按账户 ID 邀请收在折叠里。
 */
export function PrivateRoomInviteDialog({
  invites,
  onClose,
  principalId,
  room,
  rooms,
}: PrivateRoomInviteDialogProps) {
  const { t } = useTranslation();
  return (
    <Dialog
      className="private-room-invite-dialog"
      closeLabel={t('privateRooms.inviteLink.close')}
      description={t('privateRooms.inviteLink.subtitle', { room: room.name })}
      icon={<UserPlus />}
      onClose={onClose}
      title={t('privateRooms.inviteLink.title')}
    >
      <InviteLinkSection invites={invites} principalId={principalId} room={room} />
      <Details summary={t('privateRooms.inviteLink.account.summary')}>
        <PrivateRoomAccountInvite
          heading={false}
          principalId={principalId}
          room={room}
          rooms={rooms}
        />
      </Details>
    </Dialog>
  );
}

function InviteLinkSection({
  invites,
  principalId,
  room,
}: Pick<PrivateRoomInviteDialogProps, 'invites' | 'principalId' | 'room'>) {
  const { t, i18n } = useTranslation();
  const queryClient = useQueryClient();
  const key = privateRoomInviteLinkQueryKey(room.catalogId);
  const link = usePrivateRoomInviteLink(invites, room.catalogId);
  const grantable = grantableBy(room, principalId);
  const current = link.data?.ok === true ? link.data.value : null;
  // 改了权限才记下；没改时跟着现在这条链接，没有链接时用默认的（按自己有的截一下）。
  const [chosen, setChosen] = useState<PrivateRoomPermissions | null>(null);
  const shown =
    chosen ?? current?.permissions ?? intersectPermissions(DEFAULT_INVITE_PERMISSIONS, grantable);
  const [autoCopied, setAutoCopied] = useState(false);

  const create = useMutation({
    mutationFn: async () => await invites.createInviteLink(room.catalogId, shown),
    onSuccess: async (result) => {
      setAutoCopied(false);
      if (!result.ok) return;
      queryClient.setQueryData<LinkResult>(key, result);
      setChosen(null);
      // 一次点击就复制好；有的浏览器等完服务器就不认那次点击了，这时链接旁边照样有复制按钮。
      try {
        await navigator.clipboard.writeText(result.value.url);
        setAutoCopied(true);
      } catch {
        // 不算错。
      }
    },
  });
  const disable = useMutation({
    mutationFn: async () => await invites.disableInviteLink(room.catalogId),
    onSuccess: (result) => {
      if (!result.ok) return;
      setAutoCopied(false);
      queryClient.setQueryData<LinkResult>(
        key,
        err({ code: 'invite_link.not_found', retryable: false }),
      );
    },
  });
  const busy = create.isPending || disable.isPending;
  const failure =
    (create.data?.ok === false ? create.data.error : null) ??
    (disable.data?.ok === false ? disable.data.error : null) ??
    (link.data?.ok === false && !expectedLinkFailures.has(link.data.error.code)
      ? link.data.error
      : null);

  if (link.isPending) {
    return (
      <p className="private-room-invite-link__note">
        <Spinner />
        {t('privateRooms.inviteLink.checking')}
      </p>
    );
  }
  if (failure !== null && forbiddenCodes.has(failure.code)) {
    return (
      <Banner role={null} tone="info">
        {t('privateRooms.inviteLink.forbidden')}
      </Banner>
    );
  }
  const hidden = link.data?.ok === false && link.data.error.code === 'invite_link.not_revealable';
  const changed =
    chosen !== null && current !== null && !samePermissions(chosen, current.permissions);

  return (
    <section className="private-room-invite-link" aria-label={t('privateRooms.inviteLink.label')}>
      {current === null ? (
        <>
          <p className="private-room-invite-link__note">
            {t(hidden ? 'privateRooms.inviteLink.hidden' : 'privateRooms.inviteLink.none')}
          </p>
          <Button
            disabled={busy}
            icon={<Link2 aria-hidden="true" />}
            onClick={() => {
              create.mutate();
            }}
            tone="primary"
          >
            {t('privateRooms.inviteLink.create')}
          </Button>
          {disable.data?.ok === true ? (
            <p className="private-room-invite-link__note" role="status">
              {t('privateRooms.inviteLink.disabled')}
            </p>
          ) : null}
        </>
      ) : (
        <>
          {autoCopied ? (
            <Banner tone="success">{t('privateRooms.inviteLink.copied')}</Banner>
          ) : null}
          <CopyBlock
            copiedLabel={t('privateRooms.inviteLink.copied')}
            copyLabel={t('privateRooms.inviteLink.copy')}
            failedLabel={t('privateRooms.inviteLink.copyFailed')}
            text={current.url}
            textLabel={t('privateRooms.inviteLink.label')}
          />
          <p className="private-room-invite-link__note">
            {t('privateRooms.inviteLink.valid', {
              date: formatDateTime(current.expiresAtUnixMs, i18n.resolvedLanguage),
            })}
          </p>
          <div className="private-room-invite-link__actions">
            <Button
              disabled={busy}
              icon={<RefreshCw aria-hidden="true" />}
              onClick={() => {
                create.mutate();
              }}
              size="compact"
              tone="ghost"
            >
              {t('privateRooms.inviteLink.replace')}
            </Button>
            <Button
              disabled={busy}
              icon={<Link2Off aria-hidden="true" />}
              onClick={() => {
                disable.mutate();
              }}
              size="compact"
              tone="alert"
            >
              {t('privateRooms.inviteLink.disable')}
            </Button>
          </div>
          <p className="private-room-invite-link__hint">
            {t('privateRooms.inviteLink.replaceNote')}
          </p>
        </>
      )}
      {failure === null ? null : <PrivateRoomFailureNotice failure={failure} />}
      <Details summary={t('privateRooms.inviteLink.permissions.summary')}>
        <p className="private-room-invite-link__hint">
          {t('privateRooms.inviteLink.permissions.detail')}
        </p>
        <PrivateRoomCapabilityEditor
          disabled={busy}
          grantable={grantable}
          legend={t('privateRooms.inviteLink.permissions.legend')}
          onChange={setChosen}
          value={shown}
        />
        {changed ? (
          <Button
            disabled={busy}
            icon={<RefreshCw aria-hidden="true" />}
            onClick={() => {
              create.mutate();
            }}
            size="compact"
            tone="primary"
          >
            {t('privateRooms.inviteLink.permissions.apply')}
          </Button>
        ) : null}
      </Details>
    </section>
  );
}

function samePermissions(a: PrivateRoomPermissions, b: PrivateRoomPermissions): boolean {
  return privateRoomCapabilities.every(
    (capability) => a.capabilities.includes(capability) === b.capabilities.includes(capability),
  );
}
