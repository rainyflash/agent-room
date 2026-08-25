# 任务 27 验证记录：Matrix E2EE 与设备验证

## 1. 结论

任务 27 已完成。私信与私人房间现在不是“只有成员能访问”的普通 Matrix Room，而是从建房、设备信任、正文上传到历史恢复都执行明确的密码学边界：

- 私信和私人房间在创建时写入 `m.room.encryption`，使用 `m.megolm.v1.aes-sha2`；供给失败不会回退明文。
- Web 使用 Matrix JS SDK 官方 Rust Crypto、持久 IndexedDB Store 与 `OnlySignedDevicesIsolationMode`。
- Bridge 使用 `matrix-rust-sdk`、加密 SQLite Store、仅向可信设备分发房间密钥，并只接受交叉签名发送设备的解密结果。
- 安全中心提供账户加密身份、设备台账、双设备 SAS、传入验证请求、Secret Storage、密钥备份和新设备恢复。
- 私密正文上传前执行客户端 AES-256-GCM；对象存储只接收密文，解密材料只随 Matrix E2EE 事件传递。
- 成员移除后，后续消息使用新的 Megolm 会话；被移除成员既拿不到新密钥，也不能读取或枚举移除后的原始事件。
- 无加密房间、无可信设备、无备份、错误恢复口令、AEAD 认证失败或密码学初始化失败都会响亮失败，不存在明文兜底。

任务 28 才负责完整的多设备产品管理：实例列表、设备撤销、偏好同步、冲突规则和全部设备丢失后的恢复。本任务只完成 Matrix 密码学身份、验证和加密历史恢复，不伪装成完整设备管理中心。

## 2. 架构与信任边界

```text
Web 安全中心
  -> MatrixSecurityGateway
      -> Matrix JS SDK Rust Crypto
          -> IndexedDB Crypto Store
          -> Cross-signing / SAS / Secret Storage / Key Backup

Bridge 消息与交接
  -> MatrixSdkClientFactory
      -> matrix-rust-sdk
          -> 加密 SQLite Store
          -> OnlyTrustedDevices / CrossSigned
  -> MessageBodyProtectionService
      -> MessageContentCipher
          -> AES-256-GCM

Control Plane / 对象存储
  -> 只保存正文密文、摘要和有界元数据
  -> 不持有 Matrix 房间密钥或正文解密能力
```

| 事实 | 唯一权威源 | 明确禁止 |
| --- | --- | --- |
| 房间加密状态、设备密钥、交叉签名和 Megolm 会话 | Matrix / 客户端 Crypto Store | 控制平面复制密钥或自写 Olm/Megolm |
| 当前设备的恢复解锁材料 | 当前 Web 进程内存 | Local Storage、业务数据库或日志持久化恢复口令 |
| Bridge Matrix Crypto Store 口令与正文根密钥 | OS 安全存储 | 配置文件、环境日志或仓库明文 |
| 正文对象 | 对象存储中的客户端密文 | 私密房间上传可治理明文 |
| 产品房间成员关系 | PostgreSQL + Matrix 成员状态 | 仅靠 UI 隐藏实现撤权 |

依赖方向保持为 UI → 安全用例端口 → Matrix 适配器。React 不直接保存设备信任业务规则；领域快照以纯类型和纯函数计算阻断项，SDK 异常只在适配器边界映射为稳定失败码。

## 3. 房间加密与失败关闭

私人房间和直接会话的应用用例显式要求 `MatrixRoomEncryption::EndToEnd`。Matrix 供给请求在建房初始状态中写入：

- `algorithm = m.megolm.v1.aes-sha2`
- `rotation_period_ms = 604800000`
- `rotation_period_msgs = 100`

加密不是建房后的异步补丁，因此不存在房间先以明文开放、稍后才“升级”的窗口。Matrix 供给端返回加密状态，Bridge 只有在权威状态为 E2EE 时才允许私密正文路径继续；状态缺失、未知或密码学不可用都返回错误。

Web 在恢复 Matrix 会话后先启动持久 Crypto Store，再创建同步连接。Crypto 初始化失败时客户端停止并返回可诊断故障，不启动一个看似在线但实际不能加密的会话。生产构建没有测试专用安全驱动或测试房间标识。

## 4. 设备信任与 SAS 验证

安全中心从官方 SDK 投影当前用户和房间参与者的设备：设备标识、显示名、当前设备、交叉签名状态、显式验证状态和被阻断原因。当前设备只有在 SDK 报告交叉签名验证成功时才显示为“已验证”；仅存在公钥或 `signedByOwner` 不会被抬高成完整信任。

验证流程覆盖：

1. 首台设备建立交叉签名身份。
2. 第二台设备向自己的另一台设备发起验证请求。
3. 已信任设备收到独立的传入请求，可接受或拒绝。
4. 双方展示同一组 7 个 SAS Emoji；任一方选择不一致会按 `m.mismatched_sas` 取消。
5. 双方都确认后才进入已验证终态。

验证会话把 SDK 的 Requested、Ready、Started、Done 和 Cancelled 阶段映射成封闭状态机，并以可逆 `activate/deactivate` 生命周期适配 React StrictMode。组件卸载只解除订阅，不会误取消正在进行的协议；SDK 状态遗漏通知时由有界轮询对账，不建立第二套验证事实源。

## 5. Secret Storage、密钥备份与恢复

恢复设置要求当前设备已经具备交叉签名能力。客户端使用官方 SDK 从恢复口令生成恢复密钥，建立 Secret Storage 并创建新的房间密钥备份；恢复密钥只显示一次，临时私钥用后覆零。

新设备恢复时：

1. 使用恢复口令或恢复密钥解锁服务端 Secret Storage。
2. 校验密钥，不接受“解码成功”代替密码学验证。
3. 在进程内缓存最小 Secret Storage 密钥副本，替换和清理时覆零。
4. 恢复交叉签名私钥并交叉签名当前设备。
5. 从可信备份导入房间密钥，UI 显示真实 `imported / total`，零密钥不会被冒充成功。

纵向测试专门建立一次性加密房间，发送随机挑战，确认服务端看到的线事件为 `m.room.encrypted`，等待其 Megolm 密钥进入备份，再由第三台全新浏览器设备恢复。测试要求导入计数至少为 1，并使用恢复后的密钥解密同一事件、核对随机挑战。测试驱动只在 `VITE_AGENT_ROOM_VERTICAL_SECURITY_DRIVER=enabled` 的隔离验收进程中动态加载；普通生产构建经过字符串反查，未包含驱动入口、测试房间名或挑战标识。

## 6. 私密正文客户端 AEAD

Matrix E2EE 保护事件，但正文对象仍可能独立存放，因此私密正文在上传前额外执行客户端 AEAD：

- 算法标识为 `org.agentroom.content.aes-256-gcm.v1`。
- 设备正文根密钥由 OS 密码学随机源生成并存入 OS 安全存储。
- 每个 UUIDv7 提交上下文通过 HMAC-SHA-256 派生正文密钥和 96-bit nonce；同一幂等提交重试得到稳定密文，不同提交使用不同材料。
- AAD 绑定算法域、上下文标识、Matrix Room、媒体类型和明文字节数；正文摘要参与派生上下文。
- 解密材料放在 Matrix E2EE 保护的消息引用中，敏感字节使用 `Zeroizing`，Debug 输出脱敏。
- 本地消息投影需要持久化解密材料时，再用独立 XChaCha20-Poly1305 存储密钥包装正文密钥；数据库不保存裸密钥。

公共房间仍可使用服务端扫描与治理路径；私密房间的 AEAD 失败会终止提交，不会偷偷切回 `ServerSide` 明文模式。领域类型也拒绝把客户端密文伪装成已扫描明文对象。

## 7. 成员移除与密钥轮换证据

真实 Synapse 测试不是只检查“被踢后不能发言”。它执行了更强的顺序断言：

1. 房主在成员仍在房时发送一条真实加密消息，并从原始 `m.room.encrypted` 事件读取 Megolm `session_id`。
2. 服务端移除成员，房主同步到该成员的 `leave` 状态。
3. 房主发送第二条真实加密消息。
4. 第二条事件的 `session_id` 必须与第一条不同，证明 SDK 建立了新的 outbound Megolm 会话。
5. 使用被移除成员的真实 Access Token 请求第二条原始事件，只允许 `403 Forbidden` 或 `404 Not Found`。

这同时验证协议换钥与服务端历史可见性。它不声称能让已离线复制的旧明文消失；成员在移除前已合法获得的历史内容不可能通过后续换钥抹除。

## 8. 浏览器与真实依赖证据

```text
.venv\Scripts\python.exe tools\vertical.py security
  1 个真实纵向场景通过
  3 个独立浏览器设备
  首次交叉签名、双设备 SAS、正数密钥备份恢复、原事件解密通过

.venv\Scripts\python.exe tools\matrix.py test
  真实 Synapse 4/4 通过
  含 E2EE 私房、移除后 Megolm session 轮换和旧成员 403/404

just web-browser
  17 个真实 Chrome 用例通过
  含安全中心桌面、390px、SAS、恢复入口及既有大厅/私房/私信回归
```

浏览器证据：

- `artifacts/browser/task-27/security-desktop.png`
- `artifacts/browser/task-27/security-mobile.png`
- `artifacts/browser/task-27/security-verification.png`

恢复前的新设备会产生“缺少房间密钥”的 SDK 警告，这是纵向场景刻意验证的负状态；恢复完成后同一设备必须成功解密，测试才通过。环境切换期间的短暂 Sync 断线也被 SDK 重连处理，没有被错误包装成加密成功。

## 9. 质量门禁

```text
just check
  Rust fmt、Clippy -D warnings、workspace 全目标/全特性检查与测试通过
  47 个 TypeScript 测试文件、172 个测试通过
  TypeScript strict、ESLint、生产构建、协议生成/一致性通过
  526 个文本文件 Secret 扫描、11 个 Actions 固定引用通过
```

构建仅保留既有 Matrix Crypto WASM、字体和主包分块提示，Windows 本地化链接器输出，以及第三方 `proc-macro-error2` 的未来兼容提示。严格 Clippy、ESLint、类型、测试和真实依赖门禁均为零失败。

## 10. 明确边界与下一步

- 恢复口令和恢复密钥由用户负责保存；服务端不能替用户找回端到端密钥。
- 管理员和控制平面不能后台解密私信或私人房间正文；后续举报必须由客户端显式提交最小证据。
- 当前安全中心只处理 Matrix 密码学设备，不等同于 Agent 实例和产品设备的完整撤销中心。
- 下一步是任务 28：多设备同步、恢复与设备管理，包括设备/实例撤销、偏好同步、并发冲突和全部设备丢失场景。
