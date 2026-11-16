import { Banner, Button, Details } from '@agent-room/ui-system';
import { useMutation, useQueryClient } from '@tanstack/react-query';
import { Archive, Crown, LoaderCircle, LogOut, PencilLine, X } from 'lucide-react';
import { useRef, useState } from 'react';
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
  memberFor,
  permissions,
  privateRoomCapabilities,
  type PrivateRoom,
  type PrivateRoomFailure,
  type PrivateRoomGateway,
  type PrivateRoomPermissions,
} from '@/features/private-rooms/domain/private-room';
import {
  assignableRoles,
  canManageMember,
  roomRole,
  roomRoles,
} from '@/features/private-rooms/domain/room-roles';
import type { PrivateRoomListingGateway } from '@/features/private-rooms/domain/private-room-listing';
import type { PublicRoomJoinGateway } from '@/features/private-rooms/domain/public-room-join';
import type { RoomRoleChangeGateway } from '@/features/private-rooms/domain/room-role-changes';
import { ArchiveRoomConfirm } from '@/features/private-rooms/ui/archive-room-confirm';
import { PrivateRoomAccountInvite } from '@/features/private-rooms/ui/private-room-account-invite';
import { PrivateRoomCapabilityEditor } from '@/features/private-rooms/ui/private-room-capability-editor';
import { PrivateRoomFailureNotice } from '@/features/private-rooms/ui/private-room-failure-notice';
import { PrivateRoomJoinRequests } from '@/features/private-rooms/ui/private-room-join-requests';
import { RoomMemberRow } from '@/features/private-rooms/ui/room-member-row';
import { RoomRoleChanges } from '@/features/private-rooms/ui/room-role-changes';
import { useNetworkAgentIds } from '@/features/lobby/ui/network-agent-labels';
import type { Result } from '@/shared/result';

/** 服务器说这件事要最近登录过：和别的要重新登录的操作同一个码。 */
const REAUTHENTICATION_REQUIRED = 'authentication.reauthentication_required';

export type PrivateRoomMembersProps = {
  /** 这个人带了几个 Agent（房间页知道）；不给、或者回答为空就不写。 */
  readonly agentCountOf?: ((principalId: string) => number | null) | undefined;
  readonly coordinator: PrivateRoomCoordinator;
  /** 公开房间的申请；测试和夹具里可以没有，没有就不列申请。 */
  readonly joins?: PublicRoomJoinGateway | undefined;
  /** 公开设置：概况里的“可见性”和“加密”按它写；测试和夹具里可以没有。 */
  readonly listings?: PrivateRoomListingGateway | undefined;
  readonly onExitRoom: () => void;
  /** 设、撤管理员和转让房主要最近登录过：不是的话请人重新登录，登录完回到这一页。 */
  readonly onReauthenticate: () => void;
  readonly principalId: string;
  readonly recentlyAuthenticated: boolean;
  /** 身份变动记录，房主和管理员看；测试和夹具里可以没有，没有就不出现。 */
  readonly roleChanges?: RoomRoleChangeGateway | undefined;
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

type MemberNames = {
  /** 问到了昵称。 */
  readonly knows: (principalId: string) => boolean;
  readonly nameOf: (principalId: string) => string;
};

/**
 * 成员的昵称（`specs/room-people/design.md`“名字”）：只有已加入的成员问得到。还在问的时候写“…”，
 * 问不到或者没有昵称的写“名字暂时看不到”，账户 ID 在每一行的“更多”里。
 */
function useMemberNames(
  rooms: PrivateRoomGateway,
  room: PrivateRoom,
  joined: boolean,
): MemberNames {
  const { t } = useTranslation();
  const people = usePrivateRoomPeople(rooms, room.catalogId, joined);
  const names = new Map(
    people.data?.ok === true
      ? people.data.value.map((person) => [person.principalId, person.displayName.trim()])
      : [],
  );
  const known = (principalId: string): string | null => {
    const name = names.get(principalId);
    return name !== undefined && name.length > 0 ? name : null;
  };
  return {
    knows: (principalId) => known(principalId) !== null,
    nameOf: (principalId) =>
      known(principalId) ??
      (joined && people.isPending ? '…' : t('privateRooms.governance.members.unnamed')),
  };
}

type RoomCommand = {
  readonly execute: () => Promise<Result<PrivateRoom, PrivateRoomFailure>>;
  readonly onSuccess?: (room: PrivateRoom) => void;
};
/** 交出去的改动带着它是第几次。 */
type SubmittedCommand = RoomCommand & { readonly submission: number };

/**
 * 房间设置里的“成员”：房间概况（房主可以改名）、邀请、成员和身份（`specs/redesign/room-roles.md`：
 * 每人一行，身份选一下）、公开房间的申请、退出或归档。Agent 进门（敲门、进来的 Agent、口令）和公开设置是
 * 房间设置里单独的两节。
 */
export function PrivateRoomMembers({
  agentCountOf,
  coordinator,
  joins,
  listings,
  onExitRoom,
  onReauthenticate,
  principalId,
  recentlyAuthenticated,
  roleChanges,
  room,
  rooms,
}: PrivateRoomMembersProps) {
  const { t } = useTranslation();
  const queryClient = useQueryClient();
  const [failure, setFailure] = useState<PrivateRoomFailure | null>(null);
  const currentMember = memberFor(room, principalId);
  const owner = room.ownerPrincipalId === principalId;
  // 和服务器一致：邀请看“邀请”，移出、封禁、改权限看“管理”，两样各管各的。管理员那几行只有房主
  // 动得了，设、撤管理员也只有房主能做（specs/redesign/room-roles.md）。
  const canManage = owner || allows(currentMember, 'manage');
  const canInvite = owner || allows(currentMember, 'invite');
  // 身份变动记录给已加入的房主和管理员看，归档了也看得到（服务器也是这样算的）。
  const seesRoleChanges =
    currentMember?.status === 'joined' &&
    (owner || currentMember.permissions.capabilities.includes('manage'));
  const grantable = grantableBy(room, principalId);
  const [transferTarget, setTransferTarget] = useState<string | null>(null);
  const [confirmingArchive, setConfirmingArchive] = useState(false);
  const archiveButton = useRef<HTMLButtonElement>(null);
  const names = useMemberNames(rooms, room, currentMember?.status === 'joined');
  const { nameOf } = names;
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
  const submissions = useRef(0);
  const mutation = useMutation({
    mutationFn: async (command: SubmittedCommand) => await command.execute(),
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

  /** 回的是这次改动排第几次。 */
  const run = (command: RoomCommand): number => {
    setFailure(null);
    submissions.current += 1;
    mutation.mutate({ ...command, submission: submissions.current });
    return submissions.current;
  };
  // 改完到了第几次。这个数跟着改动的结果一起到，排在重新读到的名单后面：成员那一行等它到了才不再照
  // 自己刚交出去的显示。别换成在 onSettled 里另记一个状态：那样会比新名单先到，那一行闪一下旧的。
  const settled =
    mutation.variables === undefined
      ? 0
      : mutation.variables.submission - (mutation.isPending ? 1 : 0);
  // 这里只有转让会碰上每个账户 1 间不绑仓库的公开房间的限额；说明放在确认框里，别和通用的失败提示重复。
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
        <dl className="room-roles">
          {roomRoles.map((role) => (
            <div key={role}>
              <dt>{t(`privateRooms.role.${role}`)}</dt>
              <dd>{t(`privateRooms.role.${role}.detail`)}</dd>
            </div>
          ))}
        </dl>
        <ol className="room-members">
          {room.members.map((member) => {
            const self = member.principalId === principalId;
            // 网络 Agent 当房主：那一行写它的名字，那个主体不是谁的账户。
            const agentOwner =
              member.principalId === room.ownerPrincipalId && networkOwner !== null;
            const active = member.status === 'invited' || member.status === 'joined';
            const named = agentOwner || names.knows(member.principalId);
            return (
              <RoomMemberRow
                account={agentOwner ? null : member.principalId}
                agentCount={agentOwner ? null : (agentCountOf?.(member.principalId) ?? null)}
                disabled={mutation.isPending}
                grantable={grantable}
                key={member.principalId}
                manageable={canManageMember(room, principalId, member)}
                member={member}
                name={
                  agentOwner
                    ? networkOwner
                    : self && !named
                      ? t('privateRooms.governance.members.yourself')
                      : nameOf(member.principalId)
                }
                needsSignIn={!recentlyAuthenticated}
                onBan={() => {
                  run({
                    execute: async () => await rooms.ban(room.catalogId, member.principalId),
                  });
                }}
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
                onReauthenticate={onReauthenticate}
                onRemove={() => {
                  run({
                    execute: async () => await rooms.remove(room.catalogId, member.principalId),
                  });
                }}
                onTransfer={
                  owner && member.status === 'joined' && !self
                    ? () => {
                        setFailure(null);
                        setTransferTarget(member.principalId);
                      }
                    : undefined
                }
                onUnban={
                  canManage && member.status === 'banned' && !self
                    ? () => {
                        run({
                          execute: async () =>
                            await rooms.unban(room.catalogId, member.principalId),
                        });
                      }
                    : undefined
                }
                role={active ? roomRole(room, member) : null}
                roleChoices={assignableRoles(room, principalId, member)}
                self={self}
                settled={settled}
                tagged={self && named}
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
              </RoomMemberRow>
            );
          })}
        </ol>
        {room.members.some(
          (member) => member.status !== 'invited' && member.status !== 'joined',
        ) ? (
          <p className="room-members__note">{t('privateRooms.governance.members.former')}</p>
        ) : null}
      </div>

      {canManage && joins !== undefined && room.status === 'active' ? (
        <PrivateRoomJoinRequests catalogId={room.catalogId} joins={joins} listing={listing} />
      ) : null}

      {failure === null || targetHasPublicRoom ? null : failure.code ===
        REAUTHENTICATION_REQUIRED ? (
        // 页面以为还算最近登录过，服务器说不算了（两边的钟不一样、别处退出过）。
        <Banner
          action={
            <Button onClick={onReauthenticate} size="compact" tone="quiet">
              {t('privateRooms.governance.members.signInAgain')}
            </Button>
          }
          tone="warning"
        >
          {t('privateRooms.governance.members.signInFirst')}
        </Banner>
      ) : (
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

      {seesRoleChanges && roleChanges !== undefined ? (
        <RoomRoleChanges
          catalogId={room.catalogId}
          gateway={roleChanges}
          nameOf={(member) =>
            member === room.ownerPrincipalId && networkOwner !== null
              ? networkOwner
              : member === principalId && !names.knows(member)
                ? t('privateRooms.governance.members.yourself')
                : nameOf(member)
          }
        />
      ) : null}

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
