import { createContext } from 'react';

/**
 * 从对话外面打开查找：手机上顶上那一行放不下查找的按钮，它收在房间页顶栏的“更多”里。
 * 外面和对话之间隔着好几层，所以走上下文。没有它时（场景里的面板、围观页）什么都不变。
 */
export type ConversationSearchRequests = {
  /** 外面每请求一次加一。正在眼前的那一段对话看到它变了，就打开查找、把焦点放进去。 */
  readonly turn: number;
  /**
   * 正在眼前的那一段对话告诉外面此刻有没有可查的：一条消息都没有时没有查找，
   * 外面就别摆一个点了没反应的按钮。
   */
  readonly onAvailable: (available: boolean) => void;
  /**
   * 外面请这间房的对话跳到某一句（须知下面置顶那一行点的，按 Matrix 事件 ID；没加载到就往前补）。
   * `turn` 每请求一次加一，同一句点第二次也再跳一次。别的房间的对话不理；对话挂上时已经有的不算。
   */
  readonly jump?: { readonly roomId: string; readonly eventId: string; readonly turn: number };
};

export const ConversationSearchRequestContext = createContext<ConversationSearchRequests | null>(
  null,
);
