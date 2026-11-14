import { Avatar, PresenceDot, type AvatarTint } from '@agent-room/ui-system';

import type { MessageActor } from '@/features/messages/domain/message';
import { initial } from '@/shared/ui/display-name';

import { agentAvatarState } from '../domain/agent-attendance';
import type { LobbyAgent } from '../domain/lobby';
import { characterVariant } from '../scene/studio-assets';
import { AgentPortrait } from './room-illustration';

/** 四种机器人垫的浅底，顺序和 `flat-robots.png` 的四列一样：方的青、圆的红、胶囊的黄、带翅膀的青。 */
const tints: readonly AvatarTint[] = ['teal', 'coral', 'sunny', 'teal'];

/**
 * 头像上分得出来的几种状态：
 * `waiting` 在等消息（绿色实心的点），`wakes` 没在等、叫得醒（绿色的圈），
 * `idle` 连着、没在等（不画点），`offline` 离线（头像变灰）。
 */
export type AgentAvatarState = 'waiting' | 'wakes' | 'idle' | 'offline';

/** Agent 的头像：它在场景里的那个机器人，垫一块跟它同色的浅底圆。 */
export function AgentAvatar({
  id,
  state = 'idle',
  stateLabel,
  className,
}: {
  readonly id: string;
  readonly state?: AgentAvatarState;
  /** 点的那句读屏的话。旁边已经用字写了状态时不传。 */
  readonly stateLabel?: string | undefined;
  readonly className?: string | undefined;
}) {
  return (
    <Avatar
      className={className}
      dimmed={state === 'offline'}
      mark={
        state === 'waiting' || state === 'wakes' ? (
          <PresenceDot kind={state} label={stateLabel} />
        ) : undefined
      }
      tint={tints[characterVariant(id)] ?? 'teal'}
    >
      <AgentPortrait id={id} />
    </Avatar>
  );
}

/**
 * 房间里一个 Agent 的头像，带着它此刻的状态。状态旁边另有字写着，点不重复读。
 * `wakeable` 是它开着后台回复、有电脑正替它收着：没在等消息时画成绿色的圈。
 */
export function LobbyAgentAvatar({
  agent,
  now,
  wakeable = false,
}: {
  readonly agent: LobbyAgent;
  readonly now: number;
  readonly wakeable?: boolean;
}) {
  const state = agentAvatarState(agent, now);
  return (
    <AgentAvatar id={agent.agentId} state={wakeable && state !== 'waiting' ? 'wakes' : state} />
  );
}

/** 一条消息、一份资料是谁发的：Agent 画机器人，人画墨色的圆。 */
export function ActorAvatar({ actor }: { readonly actor: MessageActor }) {
  return actor.kind === 'agent' ? (
    <AgentAvatar id={actor.agentId} />
  ) : (
    <HumanAvatar name={actor.displayName} />
  );
}

/** 人的头像：墨色的圆，里面是名字的第一个字。 */
export function HumanAvatar({
  name,
  className,
}: {
  readonly name: string;
  readonly className?: string | undefined;
}) {
  return (
    <Avatar className={className} tint="ink">
      {initial(name)}
    </Avatar>
  );
}
