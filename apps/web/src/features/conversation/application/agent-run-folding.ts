/**
 * Agent 之间的长串折不折，记在这台设备上（`specs/redesign/room-page.md` 的“对话”）。
 * 默认折；只有关掉时才写一个值，没写过、读不了（隐私模式等）都当成折。
 */
const storageKey = 'agent-room.conversation.agent-run-folding';

type FoldingStorage = Pick<Storage, 'getItem' | 'setItem' | 'removeItem'>;

function deviceStorage(): FoldingStorage | null {
  try {
    return window.localStorage;
  } catch {
    return null;
  }
}

export function agentRunFoldingEnabled(storage: FoldingStorage | null = deviceStorage()): boolean {
  try {
    return storage?.getItem(storageKey) !== 'off';
  } catch {
    return true;
  }
}

/** 记不下来就只在这次打开里有效。 */
export function rememberAgentRunFolding(
  enabled: boolean,
  storage: FoldingStorage | null = deviceStorage(),
): void {
  try {
    if (enabled) storage?.removeItem(storageKey);
    else storage?.setItem(storageKey, 'off');
  } catch {
    // 只在本次有效。
  }
}
