# 找回解不开的历史消息：设计

## 背景

2026-09-28 维护者反映：房间里有 5 个 Agent、几百条消息，桌面端却显示“还没有消息”。查下来是一串连锁反应：

1. matrix-js-sdk 42.2.0 把 to-device 消息和一次性密钥数量分两次交给加密层，前一次不带数量；crypto-wasm 18.5.0 起把“没带数量”当成服务器上已经没有密钥。于是每收到一批 to-device 消息，网页端和桌面端都多传 50 个一次性密钥。维护者的桌面端设备 9 天攒了 6742 个。
2. 客户端最多只留 5000 个私钥，Synapse 又按上传先后发放。桌面端超过 5000 个以后，新来的 Agent 领到的都是客户端早已丢掉的密钥，建出来的 Olm 通道桌面端根本打不开。
3. Agent 的房间密钥都走这条坏通道，桌面端一个也没收到，房间里的消息全都解不开；界面又把解不开的消息藏了起来。

已经做了的：

- 升级 matrix-js-sdk 到 42.4.0，不再多传（#201，随下一版发布）；
- 维护者同意后，清掉了他两台设备在服务器上积压的过期一次性密钥，每台只留最新 50 个；
- 解不开的消息汇总成一条提示，不再说“还没有消息”（#202）。

还剩两件，这份设计解决它们：

- **坏通道不会自己好。** matrix-sdk-crypto 只在“已经有会话”时才替坏掉的 Olm 通道换一条新的（`mark_device_as_wedged` 先找现有会话）。桌面端和这几个 Agent 一次都没通过，没有会话可换，所以 Agent 会一直用坏通道。
- **错过的房间密钥不会重发。** Agent 认为已经发过了。Matrix 自带的密钥请求也帮不上：matrix-sdk-crypto 只向自己的其他设备要密钥，也只接受自己设备转来的密钥。

## 目标

- 人的设备发现某个 Agent 的消息因为缺密钥解不开时，自动请这个 Agent 把它自己的房间密钥重发一份；收到后导入，消息自动解开，历史也回来。
- 请求本身走一条新建的 Olm 通道，顺带换掉坏通道，之后的新密钥能正常送到。
- 人不用做任何操作；提示里说明正在找回。

## 不做

- Agent 不替别人转发密钥，只重发它自己这台设备建的会话。
- 不给不在房间里的人，也不给没经主人签名的设备。
- 发送方因为这台设备还没验证而拒绝分发（withheld）的情况不在这里处理，那要先验证设备。
- 人与人之间的消息不管：别人的客户端不认识这套请求。

## 协议

两个新的 to-device 事件，**都必须经 Olm 加密送达**，明文的一律不理。

- 字段用驼峰写法，登记进 `packages/protocol` 的 JSON Schema，由代码生成得到 TS 与 Rust 类型，和现有事件一样有合法与不合法的样例。
- 保留现有事件的 `schemaVersion`、`eventType`、`id`（UUIDv7）、`createdAt`，但不带 `actor` 与 `signature`：请求来自人的设备，人没有实例签名密钥；两头的身份都由 Olm 保证，再签一次名没有用。

### 请求：`io.github.rainyflash.agentroom.room_keys.request.v1`

人的设备发给 Agent 的某台设备：

```json
{
  "schemaVersion": "1.0",
  "eventType": "io.github.rainyflash.agentroom.room_keys.request.v1",
  "id": "0199…（UUIDv7，也是请求编号）",
  "createdAt": "2026-09-28T14:00:00Z",
  "roomId": "!abc:agentroom.chat",
  "sessionIds": ["…", "…"]
}
```

- `sessionIds` 1 到 100 个，是这个房间里这个 Agent 发的、这台设备解不开的会话。每个是 43 个字符的 base64。
- 同一个会话一小时内只请求一次。

### 应答：`io.github.rainyflash.agentroom.room_keys.v1`

Agent 的设备发回请求的那台设备：

```json
{
  "schemaVersion": "1.0",
  "eventType": "io.github.rainyflash.agentroom.room_keys.v1",
  "id": "0199…",
  "createdAt": "2026-09-28T14:00:01Z",
  "requestId": "请求的 id",
  "roomId": "!abc:agentroom.chat",
  "senderKey": "Agent 这台设备的 Curve25519",
  "senderEd25519Key": "Agent 这台设备的 Ed25519",
  "keys": [{ "sessionId": "…", "sessionKey": "…（从第 0 条起导出）" }]
}
```

- 只重发这台设备自己建的会话，所以发送方的两把公钥放在外层，每个密钥不再重复。
- `sessionKey` 是 Matrix 房间密钥导出格式里的 `session_key`，人这边照原样拼回导出格式再导入。
- 每条最多 20 个；多了分几条发，`requestId` 相同。
- 找不到的会话直接跳过，不另外说明；一个也找不到就不回。

## Agent 这边

本机 Bridge 与网络 Agent 网关都经 `matrix-adapter` 的 `restore_with_handoffs` 打开客户端，应答者就挂在这里，两边一起有：

- 和交付（`handoff.rs`）一样用 `client.add_event_handler` 收 `Raw<AnyToDeviceEvent>` 与 `Option<EncryptionInfo>`，只挑请求这一种类型。
- 处理放进单独的任务：导出要做两次 PBKDF2（见下），不能卡住同步。
- 网络 Agent 只在长轮询时同步，所以它在下一次长轮询时才回答。本机 Bridge 一直在同步，马上回答。

收到请求时依次检查，任何一条不满足就不回，只记一条去重的调试日志，不回错误，免得被人用来探测：

1. 经 Olm 加密送达，发送设备可以确定。
2. 发送设备由它的主人交叉签名。这与分发房间密钥的规则相同（[ADR 0009](../../docs/adr/0009-encryption-trust-on-first-use.md)）。
3. 发送者此刻是这个房间的成员（joined）。
4. 房间的历史可见性是 `shared` 或 `world_readable`。这时成员本来就能从服务器拿到加入前的密文，从第 0 条导出不会多给什么；其他可见性不回。生产上现有房间都是 `shared`。
5. 限流：同一台请求设备在同一个房间每 10 分钟最多答一次；每个 Agent 每分钟最多答 20 次。

然后：

- 从自己的加密存储里导出这个房间里、由自己这台设备建的（`sender_key` 是自己的 Curve25519）、请求里点名的会话。
  - matrix-sdk 0.18 没有在内存里导出的公开接口，只有 `Encryption::export_room_keys`：按条件筛会话，用口令加密后写成文件（PBKDF2 50 万轮）。
  - 所以写到临时文件，口令每次随机生成、只在内存里；再用 `decrypt_room_key_export` 读回来，随后删掉文件。文件本身是加密的，万一没删掉也读不出来。
  - Agent 自己发出的会话，SDK 也存了一份对应的入站会话，所以能导出来，而且从第 0 条开始。
- 用 `Encryption::encrypt_and_send_raw_to_device` 加密，只发给请求的那一台设备。
  - SDK 用和这台设备之间最新建的 Olm 会话加密，也就是人刚才发请求时建的那条，人那边一定解得开。

## 人这边（网页端与桌面端）

**发现。** 事件解密失败、原因是缺密钥，并且发送者在这个房间里有 Agent 在线状态事件时，记下房间、发送者、发送设备（加密内容里的 `device_id`）和会话 ID。

- 缺密钥指 #202 归为“没收到密钥”和“发在加入之前”的那几种原因码：`MEGOLM_UNKNOWN_INBOUND_SESSION_ID`、`OLM_UNKNOWN_MESSAGE_INDEX` 与 `HISTORICAL_MESSAGE_*`。发在加入之前的，Agent 会按房间的历史可见性决定给不给。
- 看在线状态事件是为了认出 Agent Room 的 Agent，不依赖用户 ID 的写法。

- 攒 2 秒发一批，同一台 Agent 设备、同一个房间合成一条请求。
- 加密内容里没有 `device_id` 时，发给这个 Agent 的每台设备。
- 同一个会话一小时内只请求一次，记在内存里。
- 这台设备自己还没由主人签名时不请求：Agent 一律不会回答，提示里已经说了要先验证这台设备。

**请求。** 用 `CryptoApi.encryptToDeviceMessages` 加密，再 `queueToDevice` 发出。

- 桌面端和这个 Agent 设备之间没有能用的 Olm 会话时，SDK 会先领对方的一次性密钥建一条新的。
- Agent 收到后，这条新会话是它和这台设备之间最新的，之后的 Olm 消息都走它，坏通道就此换掉。

**导入。** 监听 `ClientEvent.ReceivedToDeviceMessage`，应答必须全部满足：

1. 经 Olm 加密送达（`encryptionInfo` 不为空），发送设备可以确定；
2. 发送设备由主人签名，它的 Curve25519 与 `encryptionInfo` 里的一致，Ed25519 与我们查到的设备密钥一致；
3. `senderKey`、`senderEd25519Key` 就是这台发送设备的两把公钥；
4. `requestId` 是我们一小时内发给这个用户的请求，`roomId` 与请求一致，每个 `sessionId` 都在请求里。

有一条不满足就整条丢掉。满足的拼回房间密钥导出格式（`algorithm` 为 `m.megolm.v1.aes-sha2`，`sender_claimed_keys.ed25519` 取 `senderEd25519Key`，转发链为空），用 `CryptoApi.importRoomKeys` 导入。SDK 会对等着这些密钥的事件自动重试解密，提示随之更新。

**提示。** 解不开的消息里有正在找回的，就在 #202 的提示里加一句“已经请这些 Agent 重发密钥，收到后会自动解开”。

## 威胁与应对

| 威胁 | 应对 |
| --- | --- |
| 冒充人的设备骗 Agent 交出密钥 | 只认 Olm 加密送达、由主人交叉签名、此刻在房间里的设备；Olm 本身保证发送设备的身份 |
| 房间成员借此读加入前的消息 | 只在 `shared` 或 `world_readable` 的房间回答，这时服务器本来就给成员看加入前的密文 |
| 冒充 Agent 塞假密钥 | 只接受经 Olm 加密、发送设备由主人签名的应答；每个密钥的 `sender_key` 必须就是这台设备，假会话解不开真消息，也冒充不了别的发送者（SDK 解密时还会核对事件发送者与会话的设备） |
| 大量请求拖垮 Agent | 按请求设备和房间限流，按 Agent 限总数 |

## 交付计划

每一项一个 PR，CI 全绿就合并。

1. **协议**：两个新事件进 JSON Schema，重新生成 TS 与 Rust 类型，加合法与不合法样例和 Rust 一致性分派，`specs/agent-room-foundation/protocol.md` 的 to-device 表里各加一行。
2. **Agent 这边**：matrix-adapter 收请求、检查、导出、加密回发，挂在 `restore_with_handoffs`，本机 Bridge 与网络 Agent 网关一起有。
3. **人这边**：发现、请求、导入、提示。
4. **验收**：真实 Synapse、真实浏览器、真实 Bridge。人换一台新设备登录时，Agent 之前发的消息新设备本来解不开；找回后这些历史能解开，之后的新消息也能解开。这走的是同一条找回路径，比伪造一条坏通道更贴近日常。

## 运维

- 2026-09-28 清理后，服务器上所有设备的一次性密钥都不超过 60 个。#201 发布前，网页端和桌面端还会继续多传；发版前如果又有设备超过 5000 个，按同样办法清理，只留最新 50 个。

## 状态

- 2026-09-28：设计完成。维护者选择先发 Alpha 54（#201、#202），找回功能随下一版。下一步：第 1 步协议。
- 2026-09-28：第 1 步协议（#205）；第 2 步 Agent 这边：`matrix-adapter` 的应答者挂在 `restore_with_handoffs`，真实 Synapse 上“人签名之前被扣下的房间密钥，签名后请 Agent 重发、导入后解开”进了集成测试。
- 2026-09-28：第 3 步人这边：`MatrixRoomKeyRecovery` 听解密失败与 to-device 应答，攒 2 秒按 Agent 设备和房间合成请求，核对来源后导入；解不开的提示里加一句“已经请相关的 Agent 重发密钥”。实现时补了一条：这台设备没由主人签名时不请求。
