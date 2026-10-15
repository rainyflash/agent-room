import { Button } from '@agent-room/ui-system';
import { useMutation, useQueryClient } from '@tanstack/react-query';
import { UserPlus } from 'lucide-react';
import { useState } from 'react';
import { useTranslation } from 'react-i18next';

import { privateRoomListQueryKey } from '@/features/private-rooms/data/private-room-queries';
import {
  grantableBy,
  intersectPermissions,
  isPrivateRoomPrincipalId,
  type PrivateRoom,
  type PrivateRoomGateway,
  type PrivateRoomPermissions,
} from '@/features/private-rooms/domain/private-room';
import { DEFAULT_INVITE_PERMISSIONS } from '@/features/private-rooms/domain/private-room-invite-link';
import { PrivateRoomCapabilityEditor } from '@/features/private-rooms/ui/private-room-capability-editor';
import { PrivateRoomFailureNotice } from '@/features/private-rooms/ui/private-room-failure-notice';

export type PrivateRoomAccountInviteProps = {
  readonly principalId: string;
  readonly room: PrivateRoom;
  readonly rooms: PrivateRoomGateway;
  /** 放在别的标题下面时（“邀请人”对话框的折叠里）不再写自己的标题。 */
  readonly heading?: boolean;
};

/**
 * 按账户 ID 邀请：房间设置的“成员”里一份，“邀请人”对话框的折叠里一份。默认给查看、发言、自动发言，
 * 自己没有的那几项勾不上（服务器也会拒）。
 */
export function PrivateRoomAccountInvite({
  heading = true,
  principalId,
  room,
  rooms,
}: PrivateRoomAccountInviteProps) {
  const { t } = useTranslation();
  const queryClient = useQueryClient();
  const [invitee, setInvitee] = useState('');
  const grantable = grantableBy(room, principalId);
  const [invitePermissions, setInvitePermissions] = useState<PrivateRoomPermissions>(() =>
    intersectPermissions(DEFAULT_INVITE_PERMISSIONS, grantable),
  );
  const invite = useMutation({
    mutationFn: async (targetPrincipalId: string) =>
      await rooms.invite(room.catalogId, {
        permissions: invitePermissions,
        principalId: targetPrincipalId,
      }),
    onSuccess: async (result) => {
      if (!result.ok) return;
      setInvitee('');
      await queryClient.invalidateQueries({ queryKey: privateRoomListQueryKey });
    },
  });
  const failure = invite.data?.ok === false ? invite.data.error : null;
  const trimmed = invitee.trim();
  const valid = isPrivateRoomPrincipalId(trimmed);

  return (
    <form
      className="private-room-invite"
      onSubmit={(event) => {
        event.preventDefault();
        if (valid) invite.mutate(trimmed);
      }}
    >
      {heading ? (
        <div className="private-room-section-heading">
          <div>
            <h3>{t('privateRooms.governance.invite.title')}</h3>
            <p>{t('privateRooms.governance.invite.detail')}</p>
          </div>
        </div>
      ) : (
        <p className="private-room-invite-note">{t('privateRooms.governance.invite.detail')}</p>
      )}
      <label className="private-room-field">
        <span>{t('privateRooms.governance.invite.principal')}</span>
        <input
          disabled={invite.isPending}
          onChange={(event) => {
            setInvitee(event.target.value);
            if (invite.data !== undefined) invite.reset();
          }}
          placeholder={t('privateRooms.governance.invite.principalPlaceholder')}
          value={invitee}
        />
      </label>
      <PrivateRoomCapabilityEditor
        disabled={invite.isPending}
        grantable={grantable}
        legend={t('privateRooms.governance.invite.permissions')}
        onChange={setInvitePermissions}
        value={invitePermissions}
      />
      {trimmed.length > 0 && !valid ? (
        <p className="private-room-invite-hint" role="status">
          {t('privateRooms.governance.invite.principalInvalid')}
        </p>
      ) : null}
      {invite.data?.ok === true ? (
        <p className="private-room-invite-note" role="status">
          {t('privateRooms.governance.invite.sent')}
        </p>
      ) : null}
      {failure === null ? null : <PrivateRoomFailureNotice failure={failure} />}
      <Button
        disabled={invite.isPending || !valid}
        icon={<UserPlus aria-hidden="true" />}
        size="compact"
        tone="primary"
        type="submit"
      >
        {t('privateRooms.action.invite')}
      </Button>
    </form>
  );
}
