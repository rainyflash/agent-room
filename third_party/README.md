# 仓库里自带的第三方 crate

这里放的是我们打过补丁的第三方 crate，由根目录 `Cargo.toml` 的 `[patch.crates-io]` 换掉 crates.io 上的同名版本。它们不是 workspace 的成员：不跑 fmt、clippy 和它们自己的测试。

镜像只拷要用的目录，`infra` 下编 Rust 的 Containerfile 都要 `COPY third_party ./third_party`，不然 cargo 找不到补丁指的路径（`tools/tests/test_vendored_advisories.py` 卡着）。

只在升不上去、又必须补的时候才这样做；能升级就升级，升上去以后把这里的那一份删掉。

## vodozemac 0.10.0

- **为什么自带：** Matrix SDK 0.18 只认 vodozemac 0.10.x，上游的修复只发在 0.11.2。升 Matrix SDK 0.19 又卡在 SQLite 只能链接一份上（#101）。
- **从哪来：** crates.io 上的 vodozemac 0.10.0，只拷了编译要用的 `Cargo.toml`、`LICENSE`、`src/`、`benches/`，`Cargo.toml` 没改。
- **补了什么**（都是 0.11.2 里的修复，原样搬过来）：
  - 上游 #453（`d430afca`，RUSTSEC-2026-0510）：`Ed25519PublicKey::from_base64`、`Ed25519SecretKey::from_base64` 解出来不是 32 字节时返回错误，不再 panic。别人上传的设备公钥、发来的 Olm 消息里都带着这种公钥，不补的话服务器上谁都能让 Agent 的同步一解就崩，那条消息下次同步还在，Agent 就一直收不到消息。
  - 上游 #446（`acf5a0b9`，RUSTSEC-2026-0451）：Olm 根密钥棘轮的 HKDF 输出用完清零。
  - 上游 `084261d2`：消息钥匙直接展开到堆上，栈上不留副本。
  - 0.11.2 里另一处改依赖特性的修复（#448，RUSTSEC-2026-0445）只影响 0.11.0 和 0.11.1，没搬。
- **什么时候删：** 升到 Matrix SDK 0.19 以后（它用 0.11.x，`cargo update` 能拿到 0.11.2 或更新的），删掉这个目录、`Cargo.toml` 里的 `[patch.crates-io]` 和 `exclude`、`vendored.toml` 里这一段。`crates/matrix-adapter/tests/malformed_signing_keys.rs` 照样留着。

## ruma-identifiers-validation 0.12.1

- **为什么自带：** Matrix SDK 0.18 用的 ruma-common 0.19 只认 ruma-identifiers-validation 0.12.x，上游的修复只发在 0.13.0 和 0.14.0，还都改了函数的返回类型。升 Matrix SDK 0.19（它用 ruma 0.17）又卡在 SQLite 只能链接一份上（#101）。
- **从哪来：** crates.io 上的 ruma-identifiers-validation 0.12.1，只拷了 `Cargo.toml`、`CHANGELOG.md`、`src/`，`Cargo.toml` 没改。这个包上传时没带许可证文件，许可证是 MIT（`Cargo.toml` 里写着）。
- **补了什么**（返回类型照旧是 `NonZeroU8`，ruma-common 0.19 要的就是它）：
  - RUSTSEC-2026-0588（上游 `caf1ac05`）：`mxc_uri::validate` 原来把斜杠的位置截断成 `u8` 再 `unwrap`，服务器名超过 249 字节的 `mxc://` 地址一取服务器名、媒体 ID 就 panic。现在放不下的当服务器名不合法。上游是改成 `NonZeroU16`、接受到 261 字节，所以这里和上游不一样：250 字节以上的服务器名在 `mxc://` 地址里算不合法。
  - RUSTSEC-2026-0589（上游 `b2cb9b43`）：`key_id::validate` 原来先把冒号的位置截断成 `u8` 再切字符串，算法那一段超过 255 字节又带非 ASCII 字符时切到字符中间就 panic。密钥 ID 在别人上传的设备公钥和签名里，服务器上谁都能上传一份，让 Agent 查他的设备时就崩。现在照真的位置切，算法那一段超过 255 字节时返回 `MaximumLengthExceeded`（上游接受它）。
  - 两处都加了单元测试；这里不跑它们，补完时拷到别处跑过。`crates/matrix-adapter/tests/malformed_identifiers.rs` 照 Agent 真会碰到的设备公钥和 `mxc://` 地址各走一遍，只要求不 panic；换回原版 0.12.1 时三条都 panic。
- **什么时候删：** 升到 Matrix SDK 0.19 以后（ruma 0.17 用的 ruma-common 0.20 认 0.13 以上，`cargo update` 能拿到 0.14.1 或更新的），删掉这个目录、`Cargo.toml` 里 `[patch.crates-io]` 和 `exclude` 里的这一项、`vendored.toml` 里这一段。回归测试照样留着。

## 安全公告

cargo-deny 只拿 crates.io 来的包对公告，这里的它不查。`tools/vendored_advisories.py` 接在 `cargo deny check` 后面（每天的依赖安全检查和发版前的完整 CI 都跑），读 cargo-deny 刚拉下来的 RustSec 公告库：这里的 crate 只要有一条没记在 [`vendored.toml`](./vendored.toml) 里的公告就失败。

出了新公告：看它影不影响自带的这一份。影响就照上游的修复补上，记成 `patched`；不影响记成 `unaffected`，写上理由。
