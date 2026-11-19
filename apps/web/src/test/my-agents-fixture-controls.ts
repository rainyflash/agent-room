/** 仅由隔离的浏览器测试入口提供，生产应用不注册此接口。 */
export type MyAgentsFixtureControls = {
  /**
   * 这台电脑上来了一个新的 Agent 会话。接入对话框挂着人物时它把这个人物接走（像说了一句
   * “接入 Agent Room” 的 MCP Agent），没挂着时就是它自己运行了复制过去的命令。
   */
  /** `registers`：它进来后也登记了后台回复（页面要带 `?reception`），对话框就能替人打开。 */
  arriveAgent(options?: { readonly registers?: boolean }): void;
  /** 接入对话框此刻挂在连接服务上的人物；没挂着时为 null。 */
  parkedInvitation(): string | null;
  /** 像在托盘菜单里点了“更新到 X…”。 */
  requestUpdate(): void;
  /** 换账号时请原生层移出这台电脑的次数。 */
  releaseCount(): number;
  /** 每次开始登录用的意图，`default` 是没指定（普通登录）。 */
  signInIntents(): readonly string[];
  /** 交给原生层在系统浏览器里打开的地址，按先后。 */
  openedLinks(): readonly string[];
  /** 交给原生层存进“下载”文件夹的文件：存成的名字、种类和内容，按先后。 */
  savedFiles(): readonly {
    readonly fileName: string;
    readonly type: string;
    readonly text: string;
  }[];
  /** 请原生层在文件夹里指出来的文件名，按先后。 */
  shownFiles(): readonly string[];
};
export type MyAgentsFixtureWindow = Window &
  typeof globalThis & {
    readonly __agentRoomFixtureControls: MyAgentsFixtureControls;
  };
