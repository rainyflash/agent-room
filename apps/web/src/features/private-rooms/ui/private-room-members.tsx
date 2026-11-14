import { Banner, Button, Details } from '@agent-room/ui-system';
import { useMutation, useQueryClient } from '@tanstack/react-query';
import {
  Archive,
  Ban,
  Crown,
  LoaderCircle,
  LogOut,
  PencilLine,
  ShieldCheck,
  ShieldOff,
  UserCheck,
  UserMinus,
  X,
} from 'lucide-react';
import { useRef, useState, type ReactNode } from 'react';
import { useTranslation } from 'react-i18next';

import type { PrivateRoomCoordinator } from '@/features/private-rooms/application/private-room-coordinator';
import {
  privateRoomListQueryKey,
  usePrivateRoomAgentAccess,
  usePrivateRoomListing,
  usePrivateRoomOrigin,
  usePrivateRoomPeople,
} from '@/features/private-rooms/data/private-room-queries';
import {
  allows,
  grantableBy,
  isRoomAdmin,
  memberFor,
  permissions,
  privateRoomCapabilities,
  type PrivateRoom,
  type PrivateRoomFailure,
  type PrivateRoomGateway,
  type PrivateRoomMember,
  type PrivateRoomPermissions,
} from '@/features/private-rooms/domain/private-room';
import type { PrivateRoomListingGateway } from '@/features/private-rooms/domain/private-room-listing';
import type { PublicRoomJoinGateway } from '@/features/private-rooms/domain/public-room-join';
import { ArchiveRoomConfirm } from '@/features/private-rooms/ui/archive-room-confirm';
import { PrivateRoomAccountInvite } from '@/features/private-rooms/ui/private-room-account-invite';
import {
  PrivateRoomCapabilityEditor,
  toggleCapability,
} from '@/features/private-rooms/ui/private-room-capability-editor';
import { PrivateRoomFailureNotice } from '@/features/private-rooms/ui/private-room-failure-notice';
import { PrivateRoomJoinRequests } from '@/features/private-rooms/ui/private-room-join-requests';
import { useNetworkAgentIds } from '@/features/lobby/ui/network-agent-labels';
import type { Result } from '@/shared/result';

export type PrivateRoomMembersProps = {
  readonly coordinator: PrivateRoomCoordinator;
  /** 公开房间的申请；测试和夹具里可以没有，没有就不列申请。 */
  readonly joins?: PublicRoomJoinGateway | undefined;
  /** 公开设置：概况里的“可见性”和“加密”按它写；测试和夹具里可以没有。 */
  readonly listings?: PrivateRoomListingGateway | undefined;
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
 * 房间设置里的“成员”：房间概况（房主可以改名）、邀请、成员和权限（有“管理”的写“管理员”，一键设、
 * 一键撤）、公开房间的申请、退出或归档。Agent 进门（敲门、进来的 Agent、口令）和公开设置是房间设置里
 * 单独的两节。
 */
export function PrivateRoomMembers({
  coordinator,
  joins,
  listings,
  onExitRoom,
  principalId,
  recentlyAuthenticated,
  room,
  rooms,
}: PrivateRoomMembersProps) {
  const { t } = useTranslation();
  const queryClient = useQueryClient();
  const [failure, setFailure] = useState<PrivateRoomFailure | null>(null);
  const currentMember = memberFor(room, principalId);
  const owner = room.ownerPrincipalId === principalId;
  // 和服务器一致：邀请看“邀请”，移出、封禁、改权限看“管理”，两样各管各的。管理员那几行只有房主
  // 动得了，“管理”这一项也只有房主给得出、拿得掉（specs/redesign/room-roles.md）。
  const canManage = owner || allows(currentMember, 'manage');
  const canInvite = owner || allows(currentMember, 'invite');
  const grantable = grantableBy(room, principalId);
  const [transferTarget, setTransferTarget] = useState<string | null>(null);
  const [confirmingArchive, setConfirmingArchive] = useState(false);
  const archiveButton = useRef<HTMLButtonElement>(null);
  const nameOf = useMemberNames(rooms, room, currentMember?.status === 'joined');
  const relayed = useHasNetworkAgents(rooms, room, canManage);
  const origin = usePrivateRoomOrigin(rooms, room.catalogId);
  const known = origin.data?.ok === true ? origin.data.value : null;
  const listingQuery = usePrivateRoomListing(listings, room.catalogId);
  const listing = listingQuery.data?.ok === true ? listingQuery.data.value : null;
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
  // 这里只有转让会碰上每个账户 1 间公开房间的限额；说明放在确认框里，别和通用的失败提示重复。
  const targetHasPublicRoom = failure?.code === 'private_room.public_limit_reached';

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
            <dd>
              {listing?.listed === true
                ? t('privateRooms.governance.public', {
                    joinMode: t(`privateRooms.listing.joinMode.${listing.joinMode}.label`),
                  })
                : t('privateRooms.governance.inviteOnly')}
            </dd>
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
                listing?.endToEndEncrypted === false
                  ? 'privateRooms.governance.notEncrypted'
                  : known?.ownerIsNetworkAgent === true
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
        <PrivateRoomAccountInvite principalId={principalId} room={room} rooms={rooms} />
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
              admin={isRoomAdmin(room, member)}
              canManage={canManage && (owner || !isRoomAdmin(room, member))}
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
                      setFailure(null);
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
                  targetHasPublicRoom={targetHasPublicRoom}
                  name={nameOf(member.principalId)}
                  onCancel={() => {
                    setFailure(null);
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

      {canManage && joins !== undefined && room.status === 'active' ? (
        <PrivateRoomJoinRequests catalogId={room.catalogId} joins={joins} listing={listing} />
      ) : null}

      {failure === null || targetHasPublicRoom ? null : (
        <PrivateRoomFailureNotice failure={failure} />
      )}
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
          // 归档不要最近登录过（specs/cleanup-without-reauth），先在原处确认一次。
          <Button
            aria-expanded={confirmingArchive}
            disabled={mutation.isPending}
            icon={<Archive aria-hidden="true" />}
            onClick={() => {
              setConfirmingArchive((open) => !open);
            }}
            ref={archiveButton}
            size="compact"
            tone="alert"
          >
            {t('privateRooms.action.archive')}
          </Button>
        )}
        {owner && confirmingArchive ? (
          <ArchiveRoomConfirm
            name={room.name}
            onCancel={() => {
              setConfirmingArchive(false);
              archiveButton.current?.focus();
            }}
            onConfirm={() => {
              run({
                execute: async () => await rooms.archive(room.catalogId),
                onSuccess: onExitRoom,
              });
            }}
            pending={mutation.isPending}
          />
        ) : null}
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
  /** 这一行是管理员：名字后面写着，动他的事只有房主能做。 */
  readonly admin: boolean;
  /** 看的人动得了这一行：房主，或者管理员看一位不是管理员的成员。 */
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
  admin,
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
  // “管理”只有房主给得出，所以“设为管理员”“撤销管理员”只有房主看得到。
  const canAppoint = editable && grantable.capabilities.includes('manage');
  return (
    <li className={active ? undefined : 'private-room-member--inactive'}>
      <div className="private-room-member__identity">
        <span>{isOwner ? <Crown aria-hidden="true" /> : null}</span>
        <div>
          <strong>
            {name}
            {admin ? (
              <span className="private-room-member__role">
                {t('privateRooms.governance.members.admin')}
              </span>
            ) : null}
          </strong>
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
          {canAppoint ? (
            <Button
              disabled={disabled}
              icon={admin ? <ShieldOff aria-hidden="true" /> : <ShieldCheck aria-hidden="true" />}
              onClick={() => {
                onPermissionsChange(toggleCapability(member.permissions, 'manage', !admin));
              }}
              size="compact"
              tone="quiet"
            >
              {t(
                admin
                  ? 'privateRooms.governance.members.removeAdmin'
                  : 'privateRooms.governance.members.makeAdmin',
              )}
            </Button>
          ) : null}
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
  targetHasPublicRoom,
}: {
  readonly disabled: boolean;
  readonly name: string;
  readonly onCancel: () => void;
  readonly onConfirm: (kept: PrivateRoomPermissions) => void;
  readonly targetHasPublicRoom: boolean;
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
      {targetHasPublicRoom ? (
        <Banner tone="warning">
          {t('privateRooms.governance.transfer.targetHasPublicRoom', { name })}
        </Banner>
      ) : null}
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
