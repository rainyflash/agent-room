import type { AgentMessageActor, RoomMessageSignal } from '@/features/messages/domain/message';
import type { ConversationEntry } from './conversation-entries';

/** 连着这么多句都是 Agent 之间的话，才折起来。 */
export const foldedRunLength = 4;

/** 折起来的一串：Agent 之间连着说的几句。 */
export type ConversationFold = {
  readonly kind: 'fold';
  /**
   * 这一串最后一句的编号。往前补更早的消息时，一串只会从前面变长，最后一句不变，
   * 所以拿它记“这一串点开了没有”。
   */
  readonly key: string;
  readonly messages: readonly RoomMessageSignal[];
  /** 这一串里说过话的 Agent，按先后，不重复。 */
  readonly speakers: readonly AgentMessageActor[];
};
export type ConversationItem = ConversationEntry | ConversationFold;

export type FoldContext = {
  /** 已知是 Agent 的 Matrix ID：房间里的 Agent，加上说过话的 Agent。 */
  readonly agents: ReadonlySet<string>;
  /** 用来找被回复的那句是谁说的。 */
  readonly byId: ReadonlyMap<string, RoomMessageSignal>;
  /** 同一天的时间戳给出同一个值；一串不跨天，免得把“某月某日”那一行折没了。 */
  readonly dayOf?: (timestamp: number) => string;
};

/**
 * 这一句是不是 Agent 之间的话：Agent 说的，没点人、没 @所有人，也不是回复人说的话。
 * 拿不准的都当成是找人的（点的那个 ID 不认识、被回复的那句没加载到），宁可不折。
 */
export function betweenAgents(message: RoomMessageSignal, context: FoldContext): boolean {
  if (message.actor.kind !== 'agent') return false;
  if (message.preview?.mentionsEveryone === true) return false;
  if (message.preview?.conversation?.mentions.some((id) => !context.agents.has(id)) === true)
    return false;
  return (
    message.relation === undefined ||
    context.byId.get(message.relation.targetMessageId)?.actor.kind === 'agent'
  );
}

/**
 * 把 Agent 之间连着的长串换成一项 `fold`，别的原样。
 * - 一串要连着 `foldedRunLength` 句以上，中间夹了别的（人说的话、找人的话、公告改动）就断开；
 * - 最下面那一串还在进行，不折；
 * - 点了你的、回复你的本来就不算 Agent 之间的话，留在外面。
 */
export function foldAgentRuns(
  entries: readonly ConversationEntry[],
  context: FoldContext,
): readonly ConversationItem[] {
  const dayOf = context.dayOf ?? localDay;
  const items: ConversationItem[] = [];
  let run: RoomMessageSignal[] = [];
  const close = (ongoing: boolean): void => {
    if (run.length >= foldedRunLength && !ongoing) {
      const last = run.at(-1);
      if (last !== undefined)
        items.push({ kind: 'fold', key: last.messageId, messages: run, speakers: speakers(run) });
    } else for (const message of run) items.push({ kind: 'message', message });
    run = [];
  };
  for (const entry of entries) {
    if (entry.kind !== 'message' || !betweenAgents(entry.message, context)) {
      close(false);
      items.push(entry);
      continue;
    }
    const previous = run.at(-1);
    if (
      previous !== undefined &&
      dayOf(previous.serverTimestamp) !== dayOf(entry.message.serverTimestamp)
    )
      close(false);
    run.push(entry.message);
  }
  close(true);
  return items;
}

function speakers(run: readonly RoomMessageSignal[]): readonly AgentMessageActor[] {
  const seen = new Map<string, AgentMessageActor>();
  for (const message of run)
    if (message.actor.kind === 'agent' && !seen.has(message.actor.agentId))
      seen.set(message.actor.agentId, message.actor);
  return [...seen.values()];
}

function localDay(timestamp: number): string {
  return new Date(timestamp).toDateString();
}
