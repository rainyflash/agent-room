# 桌面端登录一次就好：设计

维护者 2026-10-16 在另一台电脑上装 Agent Room，点了“登录”、在浏览器里登录完，浏览器又弹出一个蓝白色的页面要他确认，点了以后回到应用才看到房间。维护者让按这份设计改，全部做完合并以后再发下一版。

本文写清：

- 现在新装一台电脑要去几趟浏览器、为什么；
- 改成什么样；
- 控制面、Bridge、桌面端各改哪里；
- 安全上多了什么、没多什么；
- 怎么分步交付。

实现以本文为准，做完一步回来更新“状态”一节。

## 现在

新装的桌面端要分三趟去系统浏览器，三样东西各管各的：

1. **账户登录**。点欢迎页的“登录”，浏览器里登录 Keycloak（桌面登录要求 5 分钟内输过密码，`max_age`），回到本机的回环地址，换成一份桌面登录（`/auth/desktop/exchange`），存进系统凭据库。
2. **聊天登录**。应用刷新以后发现没有 Matrix 登录，自动再开一次浏览器，走 Synapse 的 SSO（`desktop_begin_matrix_authentication`，`apps/desktop/src-tauri/src/matrix_session.rs`）。Keycloak 已经登录着，不用再输密码，但 Synapse 在交回登录令牌之前会停在它自己的“继续登录你的账户”确认页：回环地址不在 `sso.client_whitelist` 里，生产也没配这一项。这一页没套我们的主题，就是维护者看到的蓝白色页面。
3. **这台电脑的授权**。桌面端一启动，Bridge 就向 Keycloak 申请设备码，每个页面顶上出一条横幅“允许这台电脑连接你的 Agent”，点“打开安全登录”再去浏览器批准。设备码 10 分钟过期，过期以后桌面端停在“授权失败”，要人点重试。

问题：

- 三趟里只有第一趟是必要的。第二趟多一个看不懂的确认页；第三趟和欢迎页的“登录”同时摆在面前，不知道先点哪个。
- 批准设备码的账户和应用里登录的账户可以不是同一个，没有任何地方把两者连起来。
- 网页也有第二趟的确认页：真实网页登录验收里专门有一步 `continueThroughMatrixConsentWhenRequired` 点过去。

## 想要的样子

- **桌面端**：点“登录”，浏览器里登录一次（浏览器里已经登录着就不用输密码），回到应用就进房间，这台电脑也已经连上。不再开第二次浏览器，没有确认页，也没有设备码横幅。
- **网页**：登录以后不再停在 Synapse 的确认页。
- **照旧用设备码**：没有桌面端的 Bridge（服务器上、无头运行）和发版实机验收（`tools/release_qa.py` 起的隔离 Bridge）。桌面端要不到下面说的电脑授权时，也退回设备码。

## 一、聊天登录由控制面发

### 接口

`POST /auth/desktop/matrix-session`，没有请求体。

- 只认桌面登录：原生层代发时带的 `__Secure-agent-room-desktop-session` Cookie。网页的登录 Cookie 不认，回 401。
- 控制面以应用服务的身份替这个人登录 Matrix（`m.login.application_service`，要刷新令牌，设备名 “Agent Room Desktop”），回答和 Matrix 登录接口同一个形状：`user_id`、`device_id`、`access_token`、`refresh_token`、`expires_in_ms`。不缓存（`no-store`）。
- 不要求刚登录过。桌面端的聊天登录丢了（比如本机加密存储没了、换设备号，#302）时，也直接用它重新拿，不再跳浏览器。理由见下面“安全”一节。
- 每次调用都是 Synapse 上的一台新设备，和以前每走一次 SSO 一样。

### 第一次用 Agent Room 就是在桌面端

这个人在 Synapse 上还没有账户时（以前账户都是第一次走 SSO 时由 Synapse 建的）：

- 控制面先以应用服务的身份注册（`m.login.application_service`，`inhibit_login`）。用户名就是控制面记着的 `principal.matrix_user_id`：`user-` 加 issuer 和 sub 的摘要，和 Synapse 的登录映射器（`infra/synapse/agent_room_oidc_mapping.py`）同一条规则。
- 注册以后把显示名写成账户的显示名（以应用服务身份冒充本人 `PUT /profile/{userId}/displayname`）。
- 之后这个人在网页上走 SSO 时，映射器算出同一个用户名，Synapse 会发现它已经有了。所以 Synapse 的 OIDC 配置要加 `allow_existing_users: true`，让它认下这个账户、记上外部身份。不加的话映射器报“确定性 Matrix 用户名已被占用”，网页就登录不了。这样认下不会认错人：只有同一个 issuer 加 sub 算得出这个用户名，Agent 的用户名是 `_agent_` 开头的另一套。

### 为什么做得到

ADR 0011 的修订给应用服务注册加了一个覆盖所有本地用户的非独占命名空间（`tools/prodops/render.py`），现在只用来替本人上传签名公钥。Synapse 的应用服务注册和应用服务登录都认非独占命名空间。Agent 的 Matrix 设备一直是这样发的（`crates/matrix-provisioning-adapter` 的 `issue_device_session_internal`）。

### 网页

网页不用这个接口（它的 Cookie 不认），只去掉确认页：把网页的地址加进 Synapse 的 `sso.client_whitelist`，写成带结尾 `/` 的完整 Origin，只放行这个站点下的地址。网页的 SSO 在同一个标签页里跳一圈就回来，Keycloak 已经登录着时几乎看不到。本地和 CI 的 Synapse 配置（`tools/dev-infra.ps1`）一起改。

### 桌面端

- 网页层的 `DesktopMatrixGateway` 改成经控制面要聊天登录（用组合根注入的代发 fetch），拿到后和以前换完登录令牌一样：校验、存进系统凭据库、连上。Matrix 网关加一个接这份回答的方法，和 `#exchangeLoginToken` 共用后半段。
- 原生层的 Synapse SSO 回环（`matrix_session.rs`、`desktop_begin_matrix_authentication`）去掉。
- “等浏览器登录聊天”的提示（#314 的“重新开始登录”，夹具 `my-agents.html?matrix=signin`）在桌面端用不上了，一起去掉。出错时的“重新连接”留着。

## 二、这台电脑的授权跟着账户登录走

### 电脑授权

`POST /auth/desktop/computer-grant`，没有请求体。

- 只认桌面登录，同上。
- 回一份一次性的电脑授权：32 字节随机，10 分钟内有效。库里只存摘要，连同是谁、那份桌面登录的认证时间（`web_session.authenticated_at`）。
- 原生层自己要，自己交给 Bridge。代发通道（`desktop_control_plane_request`）不转发这个地址，网页里的代码拿不到它。

### 拿授权注册这台电脑

`POST /auth/devices/register-with-grant`，`Authorization: Bearer <电脑授权>`。

- 请求体和 `/auth/devices/register` 一样：名字、平台、签名公钥、持有证明。持有证明签的是授权的摘要，消息格式和设备码那条路签 ID Token 摘要的一样（`canonical_device_registration_message`）。
- 只能用一次：沿用设备码那条路的一次性回执（`device_authorization_receipt`）。
- 签发 10 分钟以后不收，和设备码那条路要求 ID Token 10 分钟内签发一样。
- 注册到授权里记的那个账户。账户停用、删除了就拒绝，和设备码那条路一样。
- 一年从哪天算：从那份桌面登录的认证时间算（真正输密码、或者批准设备码的那次），不从注册的时刻算。不然拿一份快满一年的登录，就能换出一台再管一年的电脑（规则见 [登录用着就不过期](../session-lifetime/design.md)）。
  - 令牌组加一列 `authenticated_at`。设备码注册的照旧空着，按 `created_at` 算；续期按它卡最晚那天。
  - 这样桌面登录和这台电脑的授权同一天到最晚，到了一起重新登录。

### Bridge 怎么拿到授权

桌面端托管启动 Bridge 时多带一个环境变量 `AGENT_ROOM_BRIDGE_DEVICE_AUTHORIZATION=supervisor`。这时没有授权的 Bridge 不申请设备码，而是：

- 报一个 `device_grant_required` 事件，等桌面端在标准输入里写一行：
  - `device-grant <电脑授权>`：拿它注册；
  - `device-code`：改走设备码，之后和今天一样。
- 注册没成就报 `device_grant_failed`（带错误码），接着等下一行。
- 标准输入本来就接着桌面端：桌面端退出前请 Bridge 先退的那一行就是从这里来的（`supervisor_released`）。现在由同一个读输入的线程分开处理三种行。旧版 Bridge 不认识的行一律跳过。

不带这个环境变量的 Bridge（服务器上无头运行的、`release_qa.py` 起的隔离 Bridge）照旧申请设备码。

### 桌面端

收到 `device_grant_required`：

- **应用里登录着**：马上去要电脑授权、写给 Bridge。界面上只有连接页那一段写“正在连上这台电脑”，没有横幅。
- **还没登录**：等着，也不出横幅。欢迎页的“登录”会顺带把这台电脑连上：桌面登录换到手以后，看到 Bridge 在等，就去要授权。
- **要授权没成，而且不是网络暂时不通**（比如控制面回 4xx）：写 `device-code`，回到今天的横幅和设备码。网络不通就按退避重试。

运行中授权丢了（在设置里移除了这台电脑、30 天没用）：照旧停下，给“重新连上这台电脑”。点了以后 Bridge 重新启动、用电脑授权一步连上，不再出设备码。不自动重连：在别的设备上移除了这台电脑，它不该自己又连回来。

## 安全

- 多出来的：系统凭据库里的桌面登录，现在能换出聊天登录和这台电脑的授权。拿到它的人，本来就拿得到同一个凭据库里的另外两样（`matrix-human-session-v1`、`device-session-v1`），所以没有多出谁能读到什么。
- 网页的登录 Cookie 两样都换不出来。网页里的代码（包括被注入的脚本）经代发通道拿得到聊天登录，但它在网页里本来就拿得到正在用的那份；拿不到电脑授权。
- 电脑授权一次性、10 分钟、只存摘要、Bridge 还要证明持有私钥。一年上限不会因为换授权被刷新。
- 服务器替人登录聊天，和 ADR 0011 已经接受的“部署方能替账户签设备”是同一件事，没有超出。ADR 0011 补一段说明。

## 不做

- 网页端用控制面发聊天登录：网页加白名单就够了，不扩大 Cookie 能换的东西。
- 去掉设备码：无头 Bridge 和发版实机验收要用，桌面端也拿它兜底。
- 应用里登录的账户和 Bridge 已经授权的账户不一致时自动换：今天也可能这样，另说。
- 在别处移除这台电脑以后自动重新连上。

## 发版

- 改了登录相关代码（`tools/release_acceptance.py` 的 `LOGIN_PATHS`），Alpha 74 的实机验收要走一次新设备授权，维护者批准设备码。
- 实机验收测的是设备码那条路（隔离 Bridge）。新的一步登录要在维护者装上 Alpha 74 以后，在另一台电脑上看一次：新装、点登录、浏览器里登录一次，回来就进房间，没有横幅、没有确认页。
- Synapse 配置改了（`allow_existing_users`、`client_whitelist`），部署时 Synapse 重启一次（`release_deploy.py` 已经会做）。
- 有迁移：电脑授权一张新表，令牌组加一列。

## 分步交付

1. 本文。
2. 控制面发聊天登录：适配器（注册、写显示名、带刷新令牌的应用服务登录），用例，接口；Synapse 配置加 `allow_existing_users` 和网页的 `client_whitelist`（`render.py`、`dev-infra.ps1`）；ADR 0011 补一段。真实 Synapse 的测试只在派发 `suite=all` 时跑。
3. 桌面端用它：`DesktopMatrixGateway` 改走控制面，去掉原生层的 SSO 回环和“等浏览器登录聊天”的提示。
4. 控制面的电脑授权：迁移，签发和注册两个接口，续期按认证时间卡。真实数据库的测试只在派发 `suite=all` 时跑。
5. Bridge：`supervisor` 模式，标准输入的两种新行，拿授权注册。
6. 桌面端：托管启动时带环境变量；收到 `device_grant_required` 就要授权、写给 Bridge；代发通道不转发电脑授权；横幅只在退回设备码时出现；“重新连上这台电脑”一步完成。浏览器验收用夹具看这几种状态。
7. 说法：README、`docs/troubleshooting.md` 里讲设备码的地方，CLAUDE.md。

第 2 到第 7 步一起随 Alpha 74 发布。第 3 步要在第 2 步之后，第 5、6 步要在第 4 步之后。

## 状态

- 2026-10-16：写好设计（#492）。
- 第 2 步 #493：控制面发聊天登录（`POST /auth/desktop/matrix-session`，`DesktopMatrixSessionService`，适配器 `human_sessions.rs`），每个人一小时最多发 10 次；Synapse 配置加 `allow_existing_users` 和网页的 `client_whitelist`；ADR 0011 补了一段。不是桌面登录时回 403 `authentication.desktop_session_required`，不回 401：桌面端见了 401 会删掉本机的登录。
- 第 3 步 #494：桌面端的 `DesktopMatrixGateway` 经代发 fetch 向控制面要聊天登录（`ControlPlaneDesktopMatrixSessions`），拿到后由 Matrix 网关的 `adoptIssuedSession` 存下、连上；原生层的 `desktop_begin_matrix_authentication` 和 `matrix_session.rs`、回环的 `/matrix/callback/` 都去掉了。“在浏览器里完成登录”的提示（`browserSignIn`、夹具 `?matrix=signin`）一起去掉：网页端登录 Matrix 时照旧算“正在连”。要不到聊天登录时说“暂时无法接通你的对话”，给“重新连接”。
- 第 4 步 #495：控制面的电脑授权。迁移 `202610160001` 加 `desktop_computer_grant` 表和令牌组的 `authenticated_at` 列；`POST /auth/desktop/computer-grant` 只认桌面登录、要窗口的 Origin，回 `{grant, expiresAtUnixMs}`；`POST /auth/devices/register-with-grant` 的请求体和设备码那条路一样（两个 `import*` 收下不用，账户已经有了），回答也一样。授权在注册的事务里删掉，删得到才往下走，照旧记一条一次性回执。用例是 `ComputerGrantService`（由 `DeviceAuthorizationService::computer_grants` 造，共用时限、签名校验和令牌规则）。满一年的登录要得到授权，但注册时回 401 `device.invalid_authorization`。
- 第 5 步 #496：Bridge 的 `supervisor` 模式（`AGENT_ROOM_BRIDGE_DEVICE_AUTHORIZATION`，还要桌面托管、开着监督事件）。标准输入改成从启动起就由一个线程按行读（`runtime/supervisor_input.rs`）：`exit` 和管道关闭照旧请 Bridge 退出，`device-grant <授权>` 拿它注册（`BridgeAuthorizationService::authorize_with_grant`，地址 `/auth/devices/register-with-grant`），`device-code` 改走设备码。等授权时报 `device_grant_required`，注册没成报 `device_grant_failed`（带错误码）接着等。授权途中（包括改走设备码以后）桌面端要退出，Bridge 不再等、正常退出。
