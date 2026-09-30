/** 仅由隔离的浏览器测试入口提供，生产应用不注册此接口。 */
export type OnboardingFixtureControls = {
  /**
   * 这台电脑上来了一个新的 Agent 会话。接入对话框挂着人物时它把这个人物接走（像说了一句
   * “接入 Agent Room” 的 MCP Agent），没挂着时就是它自己运行了复制过去的命令。
   */
  arriveAgent(): void;
  /** 接入对话框此刻挂在连接服务上的人物；没挂着时为 null。 */
  parkedInvitation(): string | null;
};
export type OnboardingFixtureWindow = Window &
  typeof globalThis & {
    readonly __agentRoomFixtureControls: OnboardingFixtureControls;
  };
