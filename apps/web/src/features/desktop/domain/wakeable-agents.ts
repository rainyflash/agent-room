import type { ReceiverView } from './reception';
import { receptionAvailability, type ReceptionRecord } from './reception-ownership';

/**
 * 这间房里此刻叫得醒的 Agent：开着后台回复，而且有电脑正替它收着消息。两处都算：
 * - 账户上记着的后台回复（哪台电脑上的都算，浏览器里也读得到），要它最近报过到；
 * - 这台电脑上正在跑的接收（只有桌面端有）。
 *
 * 只知道你自己的 Agent。别人的要服务器新给，以后再说（`specs/redesign/room-page.md`）。
 *
 * `recordsReadAtUnixMs` 是读到那份记录的时刻，不是现在：记录隔一阵才重读，拿现在去比，
 * 两次重读之间它会自己变成“联系不上”。
 */
export function wakeableAgentIds(
  roomId: string,
  records: readonly ReceptionRecord[],
  recordsReadAtUnixMs: number,
  receivers: readonly ReceiverView[],
): ReadonlySet<string> {
  const ids = new Set<string>();
  for (const record of records) {
    if (record.roomId === roomId && receptionAvailability(record, recordsReadAtUnixMs) === 'active')
      ids.add(record.agentId);
  }
  for (const view of receivers) {
    const { agentId, binding, enabled } = view.state;
    if (view.running && enabled && agentId !== null && binding.policy.roomId === roomId)
      ids.add(agentId);
  }
  return ids;
}
