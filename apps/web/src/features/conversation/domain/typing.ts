/**
 * 告诉房间里的人“我正在输入”。Agent 等消息时，叫醒它的人还在打字就再等等，
 * 免得一句话没说完就开始回（specs/agent-reading/waiting.md）。
 */
export type TypingNotifier = {
  typing(roomId: string, typing: boolean): void;
};

/** 每次说“正在输入”时请服务器最多算这么久；还在打字就提前续上。 */
export const typingTimeoutMs = 30_000;
