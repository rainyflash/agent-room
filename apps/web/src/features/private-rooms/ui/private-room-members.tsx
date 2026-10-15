import { Button, Details } from '@agent-room/ui-system';
import { useMutation, useQueryClient } from '@tanstack/react-query';
import {
  Archive,
  Ban,
  Crown,
  LoaderCircle,
  LogOut,
  PencilLine,
  UserCheck,
  UserMinus,
  UserPlus,
  X,
} from 'lucide-react';
import { useState, type ReactNode } from 'react';
import { useTranslation } from 'react-i18next';

import type { PrivateRoomCoordinator } from '@/features/private-rooms/application/private-room-coordinator';
import {
  privateRoomListQueryKey,
  usePrivateRoomAgentAccess,
  usePrivateRoomOrigin,
  usePrivateRoomPeople,
} from '@/features/private-rooms/data/private-room-queries';
import {
  allows,
  grantableBy,
  intersectPermissions,
  isPrivateRoomPrincipalId,
  memberFor,
  permissions,
  privateRoomCapabilities,
  type PrivateRoom,
  type PrivateRoomFailure,
  type PrivateRoomGateway,
  type PrivateRoomMember,
  type PrivateRoomPermissions,
} from '@/features/private-rooms/domain/private-room';
import { PrivateRoomCapabilityEditor } from '@/features/private-rooms/ui/private-room-capability-editor';
import { PrivateRoomFailureNotice } from '@/features/private-rooms/ui/private-room-failure-notice';
import { useNetworkAgentIds } from '@/features/lobby/ui/network-agent-labels';
import type { Result } from '@/shared/result';

export type PrivateRoomMembersProps = {
  readonly coordinator: PrivateRoomCoordinator;
  readonly onExitRoom: () => void;
  readonly principalId: string;
  readonly recentlyAuthenticated: boolean;
  readonly room: PrivateRoom;
  readonly rooms: PrivateRoomGateway;
};

/**
 * 房间里有没有凭口令进来的网络 Agent。网络 Agent 只能凭口令进私人房间，口令进来的名单只有管理者能看，
 * 所以别人看不到这一句，只能从成员标记上的“服务器代收发”得知。
 */
function useHasNetworkAgents(
  rooms: PrivateRoomGateway,
  room: PrivateRoom,
  canManage: boolean,
): boolean {
  const access = usePrivateRoomAgentAccess(
    rooms,
    room.catalogId,
    canManage && room.status === 'active',
  );
  const joined =
    access.data?.ok === true
      ? access.data.value.agents
          .filter((agent) => agent.status === 'joined')
          .map((agent) => agent.agentId)
      : [];
  // 缓存里记着的是所有查过的网络 Agent，不只这个房间的。
  const networkIds = useNetworkAgentIds(joined);
  return joined.some((id) => networkIds.has(id));
}

/**
 * 成员的昵称（`specs/room-people/design.md`“名字”）：只有已加入的成员问得到。还在问的时候写“…”，
 * 问不到或者没有昵称的写“名字暂时看不到”，账户 ID 在每一行的“详情”里。
 */
function useMemberNames(
  rooms: PrivateRoomGateway,
  room: PrivateRoom,
  joined: boolean,
): (principalId: string) => string {
  const { t } = useTranslation();
  const people = usePrivateRoomPeople(rooms, room.catalogId, joined);
  const names = new Map(
    people.data?.ok === true
      ? people.data.value.map((person) => [person.principalId, person.displayName.trim()])
      : [],
  );
  return (principalId) => {
    const name = names.get(principalId);
    if (name !== undefined && name.length > 0) {
      return name;
    }
    return joined && people.isPending ? '…' : t('privateRooms.governance.members.unnamed');
  };
}

type RoomCommand = {
  readonly execute: () => Promise<Result<PrivateRoom, PrivateRoomFailure>>;
  readonly onSuccess?: (room: PrivateRoom) => void;
};

/**
 * 房间设置里的“成员”：房间概况（房主可以改名）、邀请、成员和权限、退出或归档。
 * Agent 进门（敲门、进来的 Agent、口令）是房间设置里单独的一节。
 */
export function PrivateRoomMembers({
  coordinator,
  onExitRoom,
  principalId,
  recentlyAuthenticated,
  room,
  rooms,
}: PrivateRoomMembersProps) {
  const { t } = useTranslation();
  const queryClient = useQueryClient();
  const [failure, setFailure] = useState<PrivateRoomFailure | null>(null);
  const [invitee, setInvitee] = useState('');
  const currentMember = memberFor(room, principalId);
  const owner = room.ownerPrincipalId === principalId;
  // 和服务器一致：邀请看“邀请”，移出、封禁、改权限看“管理”，两样各管各的。
  const canManage = owner || allows(currentMember, 'manage');
  const canInvite = owner || allows(currentMember, 'invite');
  const grantable = grantableBy(room, principalId);
  const [invitePermissions, setInvitePermissions] = useState<PrivateRoomPermissions>(() =>
    intersectPermissions(permissions('view', 'speak'), grantable),
  );
  const [transferTarget, setTransferTarget] = useState<string | null>(null);
  const nameOf = useMemberNames(rooms, room, currentMember?.status === 'joined');
  const relayed = useHasNetworkAgents(rooms, room, canManage);
  const origin = usePrivateRoomOrigin(rooms, room.catalogId);
  const known = origin.data?.ok === true ? origin.data.value : null;
  const creator = known?.createdByAgent ?? null;
  const creatorName = creator?.displayName ?? null;
  // 网络 Agent 当房主时，成员里那一行写它的名字，不写合成主体的 ID。
  const networkOwner =
    known?.ownerIsNetworkAgent !== true
      ? null
      : creatorName === null
        ? t('privateRooms.governance.members.networkOwnerUnnamed')
        : t('privateRooms.governance.members.networkOwner', { name: creatorName });
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
    <div className="private-room-governance">
      <div className="private-room-governance__summary">
        <div>
          {owner ? (
            <RenameForm
              disabled={mutation.isPending}
              key={room.name}
              name={room.name}
              onRename={(name) => {
                run({ execute: async () => await rooms.rename(room.catalogId, name) });
              }}
            />
          ) : (
            <strong>{room.name}</strong>
          )}
        </div>
        <dl>
          {creator === null ? null : (
            <div>
              <dt>{t('privateRooms.governance.madeBy')}</dt>
              <dd>
                {creator.displayName === null
                  ? t('privateRooms.governance.madeByUnnamedAgent')
                  : t('privateRooms.governance.madeByAgent', { name: creator.displayName })}
              </dd>
            </div>
          )}
          <div>
            <dt>{t('privateRooms.governance.visibility')}</dt>
            <dd>{t('privateRooms.governance.inviteOnly')}</dd>
          </div>
          <div>
            <dt>{t('privateRooms.governance.retention')}</dt>
            <dd>
              {room.retentionDays === null
                ? t('privateRooms.governance.retentionDefault')
                : t('privateRooms.governance.retentionDays', { count: room.retentionDays })}
            </dd>
          </div>
          <div>
            <dt>{t('privateRooms.governance.encryption')}</dt>
            <dd>
              {t(
                known?.ownerIsNetworkAgent === true
                  ? 'privateRooms.governance.encryptedServerReadable'
                  : relayed
                    ? 'privateRooms.governance.encryptedRelayed'
                    : 'privateRooms.governance.encrypted',
              )}
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
            if (!isPrivateRoomPrincipalId(targetPrincipalId)) {
              return;
            }
            run({
              execute: async () =>
                await rooms.invite(room.catalogId, {
                  permissions: invitePermissions,
                  principalId: targetPrincipalId,
                }),
              onSuccess: () => {
                setInvitee('');
              },
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
              onChange={(event) => {
                setInvitee(event.target.value);
              }}
              placeholder={t('privateRooms.governance.invite.principalPlaceholder')}
              value={invitee}
            />
          </label>
          <PrivateRoomCapabilityEditor
            disabled={mutation.isPending}
            grantable={grantable}
            legend={t('privateRooms.governance.invite.permissions')}
            onChange={setInvitePermissions}
            value={invitePermissions}
          />
          {invitee.trim().length > 0 && !isPrivateRoomPrincipalId(invitee.trim()) ? (
            <p className="private-room-invite-hint" role="status">
              {t('privateRooms.governance.invite.principalInvalid')}
            </p>
          ) : null}
          <Button
            disabled={mutation.isPending || !isPrivateRoomPrincipalId(invitee.trim())}
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
              account={
                member.principalId === room.ownerPrincipalId && networkOwner !== null
                  ? null
                  : member.principalId
              }
              canManage={canManage}
              disabled={mutation.isPending}
              grantable={grantable}
              isOwner={member.principalId === room.ownerPrincipalId}
              key={member.principalId}
              member={member}
              name={
                member.principalId === principalId
                  ? t('privateRooms.governance.members.you')
                  : member.principalId === room.ownerPrincipalId && networkOwner !== null
                    ? networkOwner
                    : nameOf(member.principalId)
              }
              onBan={() => {
                run({
                  execute: async () => await rooms.ban(room.catalogId, member.principalId),
                });
              }}
              onPermissionsChange={(nextPermissions) => {
                run({
                  execute: async () =>
                    await rooms.updatePermissions(
                      room.catalogId,
                      member.principalId,
                      nextPermissions,
                    ),
                });
              }}
              onRemove={() => {
                run({
                  execute: async () => await rooms.remove(room.catalogId, member.principalId),
                });
              }}
              onTransfer={
                owner &&
                recentlyAuthenticated &&
                member.status === 'joined' &&
                member.principalId !== principalId
                  ? () => {
                      setTransferTarget(member.principalId);
                    }
                  : undefined
              }
              onUnban={
                canManage && member.status === 'banned' && member.principalId !== principalId
                  ? () => {
                      run({
                        execute: async () => await rooms.unban(room.catalogId, member.principalId),
                      });
                    }
                  : undefined
              }
              self={member.principalId === principalId}
            >
              {transferTarget === member.principalId ? (
                <TransferConfirm
                  disabled={mutation.isPending}
                  name={nameOf(member.principalId)}
                  onCancel={() => {
                    setTransferTarget(null);
                  }}
                  onConfirm={(kept) => {
                    run({
                      execute: async () =>
                        await rooms.transferOwnership(room.catalogId, {
                          formerOwnerPermissions: kept,
                          targetPrincipalId: member.principalId,
                        }),
                      onSuccess: () => {
                        setTransferTarget(null);
                      },
                    });
                  }}
                />
              ) : null}
            </MemberRow>
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
            onClick={() => {
              run({
                execute: async () => await coordinator.leave(room),
                onSuccess: onExitRoom,
              });
            }}
            size="compact"
            tone="ghost"
          >
            {t('privateRooms.action.leave')}
          </Button>
        ) : (
          <Button
            disabled={mutation.isPending || !recentlyAuthenticated}
            icon={<Archive aria-hidden="true" />}
            onClick={() => {
              run({
                execute: async () => await rooms.archive(room.catalogId),
                onSuccess: onExitRoom,
              });
            }}
            size="compact"
            title={recentlyAuthenticated ? undefined : t('privateRooms.governance.reauthRequired')}
            tone="alert"
          >
            {t('privateRooms.action.archive')}
          </Button>
        )}
      </footer>

      <Details summary={t('privateRooms.governance.details')}>
        <dl className="private-room-details">
          <div>
            <dt>{t('privateRooms.governance.roomId')}</dt>
            <dd>
              <code>{room.catalogId}</code>
            </dd>
          </div>
          <div>
            <dt>{t('privateRooms.governance.versionLabel')}</dt>
            <dd>{room.version}</dd>
          </div>
        </dl>
      </Details>
    </div>
  );
}

/** 房主改房间名；名字变了（保存成功或别处改过）就用 key 重建，草稿跟着复位。 */
function RenameForm({
  disabled,
  name,
  onRename,
}: {
  readonly disabled: boolean;
  readonly name: string;
  readonly onRename: (name: string) => void;
}) {
  const { t } = useTranslation();
  const [draft, setDraft] = useState(name);
  const next = draft.trim();
  const changed = next.length > 0 && next !== name;
  return (
    <form
      className="private-room-rename"
      onSubmit={(event) => {
        event.preventDefault();
        if (changed) onRename(next);
      }}
    >
      <label className="private-room-field">
        <span>{t('privateRooms.governance.rename.label')}</span>
        <input
          disabled={disabled}
          maxLength={128}
          onChange={(event) => {
            setDraft(event.target.value);
          }}
          value={draft}
        />
      </label>
      <Button
        disabled={disabled || !changed}
        icon={<PencilLine aria-hidden="true" />}
        size="compact"
        tone="ghost"
        type="submit"
      >
        {t('privateRooms.action.rename')}
      </Button>
    </form>
  );
}

type MemberRowProps = {
  /** 收在“详情”里的账户 ID；网络 Agent 当房主时那个主体不是谁的账户，不写。 */
  readonly account: string | null;
  readonly canManage: boolean;
  /** 这一行底下另外摆的东西：转让房主的确认。 */
  readonly children?: ReactNode;
  readonly disabled: boolean;
  /** 改权限时能加上的：房主以外的人加不了自己没有的。 */
  readonly grantable: PrivateRoomPermissions;
  readonly isOwner: boolean;
  readonly member: PrivateRoomMember;
  /** 这一行写的名字：你自己、网络 Agent 房主的名字，或者这个人的昵称。 */
  readonly name: string;
  readonly onBan: () => void;
  readonly onPermissionsChange: (permissions: PrivateRoomPermissions) => void;
  readonly onRemove: () => void;
  readonly onTransfer: (() => void) | undefined;
  readonly onUnban: (() => void) | undefined;
  readonly self: boolean;
};

function MemberRow({
  account,
  canManage,
  children,
  disabled,
  grantable,
  isOwner,
  member,
  name,
  onBan,
  onPermissionsChange,
  onRemove,
  onTransfer,
  onUnban,
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
          <strong>{name}</strong>
          <small>{t(`privateRooms.membership.${member.status}`)}</small>
        </div>
      </div>
      {account === null ? null : (
        <Details
          className="private-room-member__account"
          summary={t('privateRooms.governance.members.accountId')}
        >
          <code>{account}</code>
        </Details>
      )}
      {active ? (
        <PrivateRoomCapabilityEditor
          disabled={!editable || disabled}
          grantable={grantable}
          legend={t('privateRooms.governance.members.permissions')}
          onChange={onPermissionsChange}
          value={member.permissions}
        />
      ) : null}
      {onUnban === undefined ? null : (
        <div className="private-room-member__actions">
          <Button
            disabled={disabled}
            icon={<UserCheck aria-hidden="true" />}
            onClick={onUnban}
            size="compact"
            tone="quiet"
          >
            {t('privateRooms.action.unban')}
          </Button>
        </div>
      )}
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
      {children}
    </li>
  );
}

/**
 * 转让房主前确认一次：原房主选好自己留下哪些权限，默认留着全部（转让以后不再是房主）。
 * 房主要最近登录过才看得到“转让房主”，这里不再查。
 */
function TransferConfirm({
  disabled,
  name,
  onCancel,
  onConfirm,
}: {
  readonly disabled: boolean;
  readonly name: string;
  readonly onCancel: () => void;
  readonly onConfirm: (kept: PrivateRoomPermissions) => void;
}) {
  const { t } = useTranslation();
  const [kept, setKept] = useState<PrivateRoomPermissions>(() =>
    permissions(...privateRoomCapabilities),
  );
  return (
    <div
      className="private-room-transfer"
      role="group"
      aria-label={t('privateRooms.governance.transfer.title', { name })}
    >
      <strong>{t('privateRooms.governance.transfer.title', { name })}</strong>
      <p>{t('privateRooms.governance.transfer.detail')}</p>
      <PrivateRoomCapabilityEditor
        disabled={disabled}
        legend={t('privateRooms.governance.transfer.kept')}
        onChange={setKept}
        value={kept}
      />
      <div className="private-room-member__actions">
        <Button
          disabled={disabled}
          icon={<X aria-hidden="true" />}
          onClick={onCancel}
          size="compact"
          tone="ghost"
        >
          {t('privateRooms.action.cancel')}
        </Button>
        <Button
          disabled={disabled}
          icon={<Crown aria-hidden="true" />}
          onClick={() => {
            onConfirm(kept);
          }}
          size="compact"
          tone="alert"
        >
          {t('privateRooms.action.confirmTransfer')}
        </Button>
      </div>
    </div>
  );
}
