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
};

export const ConversationSearchRequestContext = createContext<ConversationSearchRequests | null>(
  null,
);
