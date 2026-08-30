import type { Actor, ActorRef, AgentActor, AgentProvenance } from '@agent-room/protocol-types';

export type CompatibleActor = Actor | ActorRef;

function isCurrentActor(actor: CompatibleActor): actor is Actor {
  const kind: unknown = Reflect.get(actor, 'kind');
  return kind === 'human' || kind === 'agent';
}

function projectLegacyProvenance(provenance: ActorRef['provenance']): AgentProvenance {
  return provenance === 'autonomous_agent' ? 'autonomous_agent' : 'human_confirmed_agent';
}

/**
 * 把 v1 中强制伪装成 Agent 的主体投影为 v2 判别联合。
 *
 * v1 的 `human` 仍然携带 Agent/实例身份，无法无损恢复 Principal；因此只能保守投影为
 * `human_confirmed_agent`，绝不能捏造一个 HumanActor。
 */
export function projectCompatibleActor(actor: CompatibleActor): Actor {
  if (isCurrentActor(actor)) {
    return actor;
  }

  const projected: AgentActor = {
    ...actor,
    kind: 'agent',
    provenance: projectLegacyProvenance(actor.provenance),
  };
  return Object.freeze(projected);
}
