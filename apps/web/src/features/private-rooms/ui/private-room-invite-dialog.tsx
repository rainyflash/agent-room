import { Banner, Button, CopyBlock, Details, Dialog, Spinner } from '@agent-room/ui-system';
import { useMutation, useQueryClient } from '@tanstack/react-query';
import { Link2, Link2Off, RefreshCw } from 'lucide-react';
import { useMemo, useState } from 'react';
import { useTranslation } from 'react-i18next';

import {
  privateRoomInviteLinkQueryKey,
  usePrivateRoomInviteLink,
} from '@/features/private-rooms/data/private-room-queries';
import {
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
import { invitableBy } from '@/features/private-rooms/domain/room-roles';
import { PrivateRoomAccountInvite } from '@/features/private-rooms/ui/private-room-account-invite';
import { PrivateRoomCapabilityEditor } from '@/features/private-rooms/ui/private-room-capability-editor';
import { PrivateRoomFailureNotice } from '@/features/private-rooms/ui/private-room-failure-notice';
import type { ShareCardContent } from '@/features/room-share/domain/share-card';
import { ShareCard } from '@/features/room-share/ui/share-card';
import { ShareMessageField } from '@/features/room-share/ui/share-parts';
import { formatDateTime, formatMonthDay } from '@/shared/i18n/formatters';
import { err, type Result } from '@/shared/result';
import { roomTone } from '@/shared/ui/room-tone';

export type PrivateRoomInviteDialogProps = {
  /** 邀请卡片上写的“谁邀请你”：发邀请的人的昵称。没有就不写名字。 */
  readonly inviterName?: string | undefined;
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
 * 他们登录后就能进来；能换一条、能停用。有链接时旁边是一张带二维码的邀请卡片，能复制、存下来发到聊天
 * 软件里（`specs/share-card/design.md`）。链接给的权限收在“设置”里，按账户 ID 邀请收在折叠里。
 */
export function PrivateRoomInviteDialog({
  inviterName,
  invites,
  onClose,
  principalId,
  room,
  rooms,
}: PrivateRoomInviteDialogProps) {
  const { t } = useTranslation();
  // 有链接时多一栏卡片，对话框跟着宽一档；和下面那一节是同一份查询。
  const link = usePrivateRoomInviteLink(invites, room.catalogId);
  return (
    <Dialog
      className="private-room-invite-dialog"
      closeLabel={t('privateRooms.inviteLink.close')}
      description={t('privateRooms.inviteLink.subtitle', { room: room.name })}
      onClose={onClose}
      size={link.data?.ok === true ? 'wide' : 'default'}
      title={t('privateRooms.inviteLink.title')}
    >
      <InviteLinkSection
        inviterName={inviterName}
        invites={invites}
        principalId={principalId}
        room={room}
      />
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
  inviterName,
  invites,
  principalId,
  room,
}: Pick<PrivateRoomInviteDialogProps, 'inviterName' | 'invites' | 'principalId' | 'room'>) {
  const { t, i18n } = useTranslation();
  const queryClient = useQueryClient();
  const key = privateRoomInviteLinkQueryKey(room.catalogId);
  const link = usePrivateRoomInviteLink(invites, room.catalogId);
  // 链接给不出“管理”：一条谁点了都是管理员的链接不该一勾就有（specs/redesign/room-roles.md）。
  const grantable = invitableBy(room, principalId);
  const current = link.data?.ok === true ? link.data.value : null;
  // 改了权限才记下；没改时跟着现在这条链接，没有链接时用默认的（按自己有的截一下）。
  // 以前建的链接可能带着“管理”：换一条时不再带。
  const [chosen, setChosen] = useState<PrivateRoomPermissions | null>(null);
  const shown =
    chosen ??
    (current === null
      ? intersectPermissions(DEFAULT_INVITE_PERMISSIONS, grantable)
      : withoutManage(current.permissions));
  const [autoCopied, setAutoCopied] = useState(false);
  // 一起发的那段话：改过才记下，记的是给哪条链接改的。换了一条，旧的那段带着作废的地址，不留。
  const [edited, setEdited] = useState<{ readonly url: string; readonly text: string } | null>(
    null,
  );
  const language = i18n.resolvedLanguage;
  const name = inviterName?.trim() ?? '';
  const card = useMemo<ShareCardContent | null>(
    () =>
      current === null
        ? null
        : {
            action: t('privateRooms.inviteLink.card.action'),
            lead:
              name === ''
                ? t('privateRooms.inviteLink.card.leadAnonymous')
                : t('privateRooms.inviteLink.card.lead', { name }),
            notes: [
              t('privateRooms.inviteLink.card.signIn'),
              t('privateRooms.inviteLink.card.valid', {
                date: formatMonthDay(current.expiresAtUnixMs, language),
              }),
            ],
            roomName: room.name,
            tagline: t('roomShare.card.tagline'),
            tone: roomTone(room.catalogId),
            url: current.url,
          },
    [current, language, name, room.catalogId, room.name, t],
  );

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
  // 以前建的带“管理”的链接也算“和显示的不一样”：给出“换一条”，换了才真的不带。
  const changed = current !== null && !samePermissions(shown, current.permissions);

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
          <div className="private-room-invite-link__send">
            <div className="private-room-invite-link__text">
              <CopyBlock
                copiedLabel={t('privateRooms.inviteLink.copied')}
                copyLabel={t('privateRooms.inviteLink.copy')}
                failedLabel={t('privateRooms.inviteLink.copyFailed')}
                text={current.url}
                textLabel={t('privateRooms.inviteLink.label')}
              />
              <p className="private-room-invite-link__note">
                {t('privateRooms.inviteLink.valid', {
                  date: formatDateTime(current.expiresAtUnixMs, language),
                })}
              </p>
              <ShareMessageField
                onChange={(text) => {
                  setEdited({ text, url: current.url });
                }}
                text={
                  edited?.url === current.url
                    ? edited.text
                    : `${t('privateRooms.inviteLink.message.default', { room: room.name })}\n${current.url}`
                }
              />
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
            </div>
            {card === null ? null : (
              <ShareCard
                alt={t('privateRooms.inviteLink.card.alt', { room: room.name })}
                content={card}
                detail={t('privateRooms.inviteLink.card.detail')}
                heading={t('privateRooms.inviteLink.card.title')}
              />
            )}
          </div>
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

function withoutManage(permissions: PrivateRoomPermissions): PrivateRoomPermissions {
  return {
    capabilities: permissions.capabilities.filter((capability) => capability !== 'manage'),
  };
}

function samePermissions(a: PrivateRoomPermissions, b: PrivateRoomPermissions): boolean {
  return privateRoomCapabilities.every(
    (capability) => a.capabilities.includes(capability) === b.capabilities.includes(capability),
  );
}
