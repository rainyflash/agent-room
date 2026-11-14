import { Avatar, Button } from '@agent-room/ui-system';
import { Ban, Crown, Ellipsis, UserCheck, UserMinus } from 'lucide-react';
import { useId, useState, type ReactNode } from 'react';
import { useTranslation } from 'react-i18next';

import type {
  PrivateRoomMember,
  PrivateRoomPermissions,
} from '@/features/private-rooms/domain/private-room';
import {
  crossesAdmin,
  permissionsForRole,
  roleOf,
  type AssignableRole,
  type RoomRole,
} from '@/features/private-rooms/domain/room-roles';
import { toggleCapability } from '@/features/private-rooms/ui/private-room-capability-editor';
import { initial } from '@/shared/ui/display-name';

import './room-member-row.css';

export type RoomMemberRowProps = {
  /** 收在“更多”里的账户 ID；网络 Agent 当房主时那个主体不是谁的账户，不写。 */
  readonly account: string | null;
  /** 他带了几个 Agent；不知道时为空，不写。 */
  readonly agentCount: number | null;
  /** 这一行底下另外摆的东西：转让房主的确认。 */
  readonly children?: ReactNode;
  /** 名单正在改（谁那一行的都算）：服务器答完、名单也重新读过以后才放开。 */
  readonly disabled: boolean;
  /** 改权限时能加上的：房主以外的人加不了自己没有的。 */
  readonly grantable: PrivateRoomPermissions;
  /** 看的人动得了这一行的两个开关和移出、封禁。 */
  readonly manageable: boolean;
  readonly member: PrivateRoomMember;
  /** 这一行写的名字：昵称，或者网络 Agent 房主的名字。 */
  readonly name: string;
  /** 看的人不算最近登录过：设、撤管理员和转让房主先请他重新登录。 */
  readonly needsSignIn: boolean;
  readonly onBan: () => void;
  /** 改这一行的权限。回的是这次改动在名单里排第几次（和 `settled` 比）。 */
  readonly onPermissionsChange: (permissions: PrivateRoomPermissions) => number;
  readonly onReauthenticate: () => void;
  readonly onRemove: () => void;
  /** 看的人能把房主转给这一行的人。 */
  readonly onTransfer: (() => void) | undefined;
  readonly onUnban: (() => void) | undefined;
  /** 服务器记着的身份；不在房间里的人（退出、移出、封禁）没有。 */
  readonly role: RoomRole | null;
  /** 看的人能给这一行换成的身份（含现在的）；少于两种就只写字。 */
  readonly roleChoices: readonly AssignableRole[];
  /** 这一行是看的人自己：换自己的身份（管理员不当了）先确认一次。 */
  readonly self: boolean;
  /** 名单的改动改完到了第几次：服务器答完、新名单也到了以后才往上走。 */
  readonly settled: number;
  /** 名字后面标一个“你”：这一行是你、名字写的是昵称时。 */
  readonly tagged: boolean;
};

/** 选了身份、点了转让以后先问一句的两种情况。 */
type Question = 'sign-in' | 'step-down';

type Sent = { readonly permissions: PrivateRoomPermissions; readonly submission: number };

/**
 * 房间设置“成员”里的一行（`specs/redesign/room-roles.md`）：名字、身份、带了几个 Agent，“更多”里是
 * 两个开关（带来的 Agent 能说话、能邀请人）、移出和封禁、账户 ID。身份是选一下：
 * 设、撤管理员要最近登录过，不是的话先请人重新登录；管理员把自己降成成员先确认一次。
 *
 * 选了、拨了以后这一行马上照新的显示，不等服务器：服务器答完还要重新读一遍名单，等那两趟的工夫
 * 选择框会先跳回旧的。名单说这一次改完了（`settled` 到了这一次，这时新名单已经到了）才换回照服务器
 * 记着的显示；没改成就是回到原样，失败的原因写在名单下面。
 */
export function RoomMemberRow({
  account,
  agentCount,
  children,
  disabled: listBusy,
  grantable,
  manageable,
  member,
  name,
  needsSignIn,
  onBan,
  onPermissionsChange,
  onReauthenticate,
  onRemove,
  onTransfer,
  onUnban,
  role: recorded,
  roleChoices,
  self,
  settled,
  tagged,
}: RoomMemberRowProps) {
  const { t } = useTranslation();
  const id = useId();
  const [expanded, setExpanded] = useState(false);
  const [question, setQuestion] = useState<Question | null>(null);
  // 这一行最近交给服务器的那一份权限，和它是名单的第几次改动。
  const [last, setLast] = useState<Sent | null>(null);
  // 还没改完的才算数。
  const sent = last !== null && last.submission > settled ? last.permissions : null;
  const shown = sent ?? member.permissions;
  // 名单说“正在改”要晚一拍才到；这一行自己交出去的那一刻就先锁上，免得紧接着的一下照旧的权限又改一次。
  const disabled = listBusy || sent !== null;
  const role = sent === null || recorded === null || recorded === 'owner' ? recorded : roleOf(sent);
  const change = (next: PrivateRoomPermissions): void => {
    setLast({ permissions: next, submission: onPermissionsChange(next) });
  };
  const has = (capability: 'automate' | 'invite'): boolean =>
    shown.capabilities.includes(capability);
  const switches = role !== null && role !== 'owner';
  const hasMore =
    switches || manageable || onTransfer !== undefined || onUnban !== undefined || account !== null;

  const chooseRole = (next: AssignableRole): void => {
    if (role === null || next === role) return;
    if (self) {
      setQuestion('step-down');
      return;
    }
    if (crossesAdmin(role, next) && needsSignIn) {
      setQuestion('sign-in');
      return;
    }
    setQuestion(null);
    change(permissionsForRole(member.permissions, next, grantable));
  };

  return (
    <li className="room-member" data-inactive={role === null ? '' : undefined}>
      <div className="room-member__row">
        <div className="room-member__who">
          <Avatar dimmed={role === null} tint="ink">
            {role === 'owner' ? <Crown /> : initial(name)}
          </Avatar>
          <div>
            <strong>
              {name}
              {tagged ? (
                <span className="room-member__you">
                  {' '}
                  {t('privateRooms.governance.members.you')}
                </span>
              ) : null}
            </strong>
            {member.status === 'invited' ? (
              <small>{t('privateRooms.membership.invited')}</small>
            ) : null}
          </div>
        </div>
        <div className="room-member__role">
          {role === null ? (
            <span className="room-member__status">
              {t(`privateRooms.membership.${member.status}`)}
            </span>
          ) : roleChoices.length > 1 && role !== 'owner' ? (
            <select
              aria-label={t('privateRooms.governance.members.roleOf', { name })}
              disabled={disabled}
              onChange={(event) => {
                const next = roleChoices.find((choice) => choice === event.target.value);
                if (next !== undefined) chooseRole(next);
              }}
              value={role}
            >
              {roleChoices.map((choice) => (
                <option key={choice} value={choice}>
                  {t(`privateRooms.role.${choice}`)}
                </option>
              ))}
            </select>
          ) : (
            <span>{t(`privateRooms.role.${role}`)}</span>
          )}
        </div>
        <span className="room-member__agents">
          {agentCount === null || role === null
            ? null
            : agentCount === 0
              ? t('privateRooms.governance.members.noAgents')
              : t('privateRooms.governance.members.agents', { count: agentCount })}
        </span>
        {hasMore ? (
          <button
            aria-controls={`${id}-more`}
            aria-expanded={expanded}
            aria-label={t('privateRooms.governance.members.more', { name })}
            className="room-member__more"
            onClick={() => {
              setExpanded((open) => !open);
            }}
            type="button"
          >
            <Ellipsis aria-hidden="true" />
          </button>
        ) : (
          <span />
        )}
      </div>

      {question === null ? null : (
        <div aria-labelledby={`${id}-question`} className="room-member__question" role="group">
          <p id={`${id}-question`} role="status">
            {t(
              question === 'sign-in'
                ? 'privateRooms.governance.members.signInFirst'
                : 'privateRooms.governance.members.stepDown',
            )}
          </p>
          <div className="room-member__actions">
            <Button
              disabled={disabled}
              onClick={() => {
                setQuestion(null);
              }}
              size="compact"
              tone="ghost"
            >
              {t('privateRooms.action.cancel')}
            </Button>
            {question === 'sign-in' ? (
              <Button onClick={onReauthenticate} size="compact" tone="primary">
                {t('privateRooms.governance.members.signInAgain')}
              </Button>
            ) : (
              <Button
                disabled={disabled}
                onClick={() => {
                  setQuestion(null);
                  change(permissionsForRole(member.permissions, 'member', grantable));
                }}
                size="compact"
                tone="alert"
              >
                {t('privateRooms.governance.members.stepDown.confirm')}
              </Button>
            )}
          </div>
        </div>
      )}

      {expanded && hasMore ? (
        <div aria-label={name} className="room-member__panel" id={`${id}-more`} role="group">
          {switches ? (
            <div className="room-member__switches">
              <label>
                <input
                  checked={has('automate')}
                  disabled={
                    disabled ||
                    !manageable ||
                    role === 'viewer' ||
                    (!has('automate') && !grantable.capabilities.includes('automate'))
                  }
                  onChange={(event) => {
                    change(toggleCapability(member.permissions, 'automate', event.target.checked));
                  }}
                  role="switch"
                  type="checkbox"
                />
                <span>{t('privateRooms.governance.members.agentsSpeak')}</span>
              </label>
              <label>
                <input
                  checked={has('invite')}
                  disabled={
                    disabled ||
                    !manageable ||
                    role === 'viewer' ||
                    (!has('invite') && !grantable.capabilities.includes('invite'))
                  }
                  onChange={(event) => {
                    change(toggleCapability(member.permissions, 'invite', event.target.checked));
                  }}
                  role="switch"
                  type="checkbox"
                />
                <span>{t('privateRooms.governance.members.canInvite')}</span>
              </label>
            </div>
          ) : null}
          {manageable || onTransfer !== undefined || onUnban !== undefined ? (
            <div className="room-member__actions">
              {onUnban === undefined ? null : (
                <Button
                  disabled={disabled}
                  icon={<UserCheck aria-hidden="true" />}
                  onClick={onUnban}
                  size="compact"
                  tone="quiet"
                >
                  {t('privateRooms.action.unban')}
                </Button>
              )}
              {onTransfer === undefined ? null : (
                <Button
                  disabled={disabled}
                  icon={<Crown aria-hidden="true" />}
                  onClick={() => {
                    if (needsSignIn) setQuestion('sign-in');
                    else onTransfer();
                  }}
                  size="compact"
                  tone="quiet"
                >
                  {t('privateRooms.action.transfer')}
                </Button>
              )}
              {manageable ? (
                <>
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
                </>
              ) : null}
            </div>
          ) : null}
          {account === null ? null : (
            <p className="room-member__account">
              {t('privateRooms.governance.members.accountId')} <code>{account}</code>
            </p>
          )}
        </div>
      ) : null}
      {children}
    </li>
  );
}
