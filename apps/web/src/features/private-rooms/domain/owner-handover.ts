import { z } from 'zod';

/**
 * 房主删除了账户、房间交给了当管理员最久的那位（`specs/redesign/room-roles.md` 的“房主不在了（R6）”）。
 * 删除任务以应用服务把这条写进房间状态；房间的 `state_default` 只有应用服务够得着，所以房间状态里有的
 * 就是删除任务写的。
 */
export const roomOwnerHandoverEventType = 'io.github.rainyflash.agentroom.room.owner_handover.v1';

const matrixUserIdSchema = z
  .string()
  .min(4)
  .max(255)
  .regex(/^@[^\s:]+:\S+$/u);

const ownerHandoverStateSchema = z.looseObject({
  eventType: z.literal(roomOwnerHandoverEventType),
  owner: matrixUserIdSchema,
  formerOwner: matrixUserIdSchema,
});

/** 状态事件里接手的人是谁。读不出来的不算换了房主。 */
export function parseRoomOwnerHandover(content: unknown): { readonly owner: string } | undefined {
  const parsed = ownerHandoverStateSchema.safeParse(content);
  return parsed.success ? { owner: parsed.data.owner } : undefined;
}
