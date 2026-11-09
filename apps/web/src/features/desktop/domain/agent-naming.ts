/** 网络 Agent 的名字最多 64 个字符，本机的更长；按最短的来，写进话里的名字三种方式都收。 */
export const MAX_AGENT_NAME_CHARACTERS = 64;

/**
 * 人在接入对话框里替 Agent 起的名字：控制字符换成空格，免得把发给 Agent 的话拆开，服务器也不收；
 * 首尾空白去掉。结果是空的就是没起名，Agent 自己起。
 */
export function cleanAgentName(value: string): string {
  return value.replace(/\p{Cc}+/gu, ' ').trim();
}
