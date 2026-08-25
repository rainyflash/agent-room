import { Button } from '@agent-room/ui-system';
import { useMutation, useQueryClient } from '@tanstack/react-query';
import { Archive, Ban, Crown, LoaderCircle, LogOut, UserMinus, UserPlus } from 'lucide-react';
import { useState } from 'react';
import { useTranslation } from 'react-i18next';

import type { PrivateRoomCoordinator } from '@/features/private-rooms/application/private-room-coordinator';
import { privateRoomListQueryKey } from '@/features/private-rooms/data/private-room-queries';
import {
  allows,
  memberFor,
  permissions,
  type PrivateRoom,
  type PrivateRoomFailure,
  type PrivateRoomGateway,
  type PrivateRoomMember,
  type PrivateRoomPermissions,
} from '@/features/private-rooms/domain/private-room';
import { PrivateRoomCapabilityEditor } from '@/features/private-rooms/ui/private-room-capability-editor';
import { PrivateRoomFailureNotice } from '@/features/private-rooms/ui/private-room-create-flow';
import type { Result } from '@/shared/result';

export type PrivateRoomGovernanceProps = {
  readonly coordinator: PrivateRoomCoordinator;
  readonly onExitRoom: () => void;
  readonly principalId: string;
  readonly recentlyAuthenticated: boolean;
  readonly room: PrivateRoom;
  readonly rooms: PrivateRoomGateway;
};

type RoomCommand = {
  readonly execute: () => Promise<Result<PrivateRoom, PrivateRoomFailure>>;
  readonly onSuccess?: (room: PrivateRoom) => void;
};

export function PrivateRoomGovernance({
  coordinator,
  onExitRoom,
  principalId,
  recentlyAuthenticated,
  room,
  rooms,
}: PrivateRoomGovernanceProps) {
  const { t } = useTranslation();
  const queryClient = useQueryClient();
  const [failure, setFailure] = useState<PrivateRoomFailure | null>(null);
  const [invitee, setInvitee] = useState('');
  const [invitePermissions, setInvitePermissions] = useState<PrivateRoomPermissions>(
    permissions('view', 'speak'),
  );
  const currentMember = memberFor(room, principalId);
  const owner = room.ownerPrincipalId === principalId;
  const canManage = owner || allows(currentMember, 'manage');
  const canInvite = canManage || allows(currentMember, 'invite');
  const mutation = useMutation({
    mutationFn: async (command: RoomCommand) => await command.execute(),
    onSuccess: async (result, command) => {
      if (!result.ok) {
        setFailure(result.error);
        return;
      }
      setFailure(null);
      await queryClient.invalidateQueries({ queryKey: privateRoomListQueryKey });
      command.onSuccess?.(result.value);
    },
  });

  const run = (command: RoomCommand): void => {
    setFailure(null);
    mutation.mutate(command);
  };

  return (
    <section className="private-room-governance" aria-labelledby="private-room-governance-title">
      <header>
        <div>
          <p className="eyebrow">{t('privateRooms.governance.eyebrow')}</p>
          <h2 id="private-room-governance-title">{t('privateRooms.governance.title')}</h2>
        </div>
        <span>{t('privateRooms.governance.version', { version: room.version })}</span>
      </header>

      <div className="private-room-governance__summary">
        <div>
          <strong>{room.name}</strong>
          <span>{room.catalogId}</span>
        </div>
        <dl>
          <div>
            <dt>{t('privateRooms.governance.visibility')}</dt>
            <dd>{t('privateRooms.governance.inviteOnly')}</dd>
          </div>
          <div>
            <dt>{t('privateRooms.governance.retention')}</dt>
            <dd>
              {room.retentionDays === null
                ? t('privateRooms.governance.retentionDefault')
                : t('privateRooms.create.retentionDays', { count: room.retentionDays })}
            </dd>
          </div>
        </dl>
      </div>

      {canInvite ? (
        <form
          className="private-room-invite"
          onSubmit={(event) => {
            event.preventDefault();
            const targetPrincipalId = invitee.trim();
            if (targetPrincipalId.length === 0) {
              return;
            }
            run({
              execute: async () =>
                await rooms.invite(room.catalogId, {
                  permissions: invitePermissions,
                  principalId: targetPrincipalId,
                }),
              onSuccess: () => setInvitee(''),
            });
          }}
        >
          <div className="private-room-section-heading">
            <div>
              <h3>{t('privateRooms.governance.invite.title')}</h3>
              <p>{t('privateRooms.governance.invite.detail')}</p>
            </div>
          </div>
          <label className="private-room-field">
            <span>{t('privateRooms.governance.invite.principal')}</span>
            <input
              disabled={mutation.isPending}
              onChange={(event) => setInvitee(event.target.value)}
              placeholder={t('privateRooms.governance.invite.principalPlaceholder')}
              value={invitee}
            />
          </label>
          <PrivateRoomCapabilityEditor
            disabled={mutation.isPending}
            legend={t('privateRooms.governance.invite.permissions')}
            onChange={setInvitePermissions}
            value={invitePermissions}
          />
          <Button
            disabled={mutation.isPending || invitee.trim().length === 0}
            icon={<UserPlus aria-hidden="true" />}
            size="compact"
            tone="primary"
            type="submit"
          >
            {t('privateRooms.action.invite')}
          </Button>
        </form>
      ) : null}

      <div className="private-room-members">
        <div className="private-room-section-heading">
          <div>
            <h3>{t('privateRooms.governance.members.title')}</h3>
            <p>{t('privateRooms.governance.members.detail')}</p>
          </div>
          <span>{room.members.length}</span>
        </div>
        <ol>
          {room.members.map((member) => (
            <MemberRow
              canManage={canManage}
              disabled={mutation.isPending}
              isOwner={member.principalId === room.ownerPrincipalId}
              key={member.principalId}
              member={member}
              onBan={() =>
                run({
                  execute: async () => await rooms.ban(room.catalogId, member.principalId),
                })
              }
              onPermissionsChange={(nextPermissions) =>
                run({
                  execute: async () =>
                    await rooms.updatePermissions(
                      room.catalogId,
                      member.principalId,
                      nextPermissions,
                    ),
                })
              }
              onRemove={() =>
                run({
                  execute: async () => await rooms.remove(room.catalogId, member.principalId),
                })
              }
              onTransfer={
                owner &&
                recentlyAuthenticated &&
                member.status === 'joined' &&
                member.principalId !== principalId
                  ? () =>
                      run({
                        execute: async () =>
                          await rooms.transferOwnership(room.catalogId, {
                            formerOwnerPermissions: permissions('view', 'speak'),
                            targetPrincipalId: member.principalId,
                          }),
                      })
                  : undefined
              }
              self={member.principalId === principalId}
            />
          ))}
        </ol>
      </div>

      {failure === null ? null : <PrivateRoomFailureNotice failure={failure} />}
      {mutation.isPending ? (
        <p className="private-room-operation" role="status">
          <LoaderCircle aria-hidden="true" className="private-room-spin" />
          {t('privateRooms.governance.applying')}
        </p>
      ) : null}

      <footer className="private-room-danger-zone">
        <div>
          <h3>{t('privateRooms.governance.access.title')}</h3>
          <p>{t('privateRooms.governance.access.detail')}</p>
        </div>
        {!owner ? (
          <Button
            disabled={mutation.isPending}
            icon={<LogOut aria-hidden="true" />}
            onClick={() =>
              run({
                execute: async () => await coordinator.leave(room),
                onSuccess: onExitRoom,
              })
            }
            size="compact"
            tone="ghost"
          >
            {t('privateRooms.action.leave')}
          </Button>
        ) : (
          <Button
            disabled={mutation.isPending || !recentlyAuthenticated}
            icon={<Archive aria-hidden="true" />}
            onClick={() =>
              run({
                execute: async () => await rooms.archive(room.catalogId),
                onSuccess: onExitRoom,
              })
            }
            size="compact"
            title={recentlyAuthenticated ? undefined : t('privateRooms.governance.reauthRequired')}
            tone="alert"
          >
            {t('privateRooms.action.archive')}
          </Button>
        )}
      </footer>
    </section>
  );
}

type MemberRowProps = {
  readonly canManage: boolean;
  readonly disabled: boolean;
  readonly isOwner: boolean;
  readonly member: PrivateRoomMember;
  readonly onBan: () => void;
  readonly onPermissionsChange: (permissions: PrivateRoomPermissions) => void;
  readonly onRemove: () => void;
  readonly onTransfer: (() => void) | undefined;
  readonly self: boolean;
};

function MemberRow({
  canManage,
  disabled,
  isOwner,
  member,
  onBan,
  onPermissionsChange,
  onRemove,
  onTransfer,
  self,
}: MemberRowProps) {
  const { t } = useTranslation();
  const active = member.status === 'invited' || member.status === 'joined';
  const editable = canManage && active && !isOwner && !self;
  return (
    <li className={active ? undefined : 'private-room-member--inactive'}>
      <div className="private-room-member__identity">
        <span>{isOwner ? <Crown aria-hidden="true" /> : null}</span>
        <div>
          <strong>{self ? t('privateRooms.governance.members.you') : member.principalId}</strong>
          <small>{t(`privateRooms.membership.${member.status}`)}</small>
        </div>
      </div>
      {active ? (
        <PrivateRoomCapabilityEditor
          disabled={!editable || disabled}
          legend={t('privateRooms.governance.members.permissions')}
          onChange={onPermissionsChange}
          value={member.permissions}
        />
      ) : null}
      {editable ? (
        <div className="private-room-member__actions">
          {onTransfer === undefined ? null : (
            <Button
              disabled={disabled}
              icon={<Crown aria-hidden="true" />}
              onClick={onTransfer}
              size="compact"
              tone="quiet"
            >
              {t('privateRooms.action.transfer')}
            </Button>
          )}
          <Button
            disabled={disabled}
            icon={<UserMinus aria-hidden="true" />}
            onClick={onRemove}
            size="compact"
            tone="quiet"
          >
            {t('privateRooms.action.remove')}
          </Button>
          <Button
            disabled={disabled}
            icon={<Ban aria-hidden="true" />}
            onClick={onBan}
            size="compact"
            tone="quiet"
          >
            {t('privateRooms.action.ban')}
          </Button>
        </div>
      ) : null}
    </li>
  );
}
