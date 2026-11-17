# 预览图用的字体

控制面画分享预览图（`/r/<分享码>/preview.png`、`/r/<分享码>/s/<段码>/preview.png`，见 [设计](../../../../specs/redesign/project-rooms.md) 的“预览图”）只用这一份字体：思源黑体的简体中文粗体（Noto Sans SC Bold），里面也有拉丁字母，不另带别的字体。

- 文件：`NotoSansSC-Bold.otf`，8,543,168 字节，SHA-256 `c6cb5a93abaa9edc8ee7463b7ebb7f42d618d40e6ed2f7a5371c97b0b64767c0`。
- 来源：<https://github.com/notofonts/noto-cjk> 的标签 `Sans2.004`，路径 `Sans/SubsetOTF/SC/NotoSansSC-Bold.otf`（Git blob `bcc11bc3…` 和那个标签里的一致）。
- 许可：SIL Open Font License 1.1，原文是同一个标签根目录的 `LICENSE`，原样放在这里（SHA-256 `6a73f9541c2de74158c0e7cf6b0a58ef774f5a780bf191f2d7ec9cc53efe2bf2`）。

只带一种粗细，图上每一行都用它，靠字号和颜色分主次，镜像只大这一个文件。

控制面不把字体编进程序，启动后第一次画图时从 `AGENT_ROOM_PREVIEW_FONT` 读（默认 `/usr/share/agent-room/fonts/NotoSansSC-Bold.otf`，`infra/production/Containerfile.control-plane` 把这两个文件拷到那里）。读不到就记一条告警，预览图一律跳到网页上的通用图。

换字体、换版本时这几处一起改：这里的说明和摘要、`tools/license_inventory.py` 里“Bundled data”那一段（再跑 `python tools/license_inventory.py generate`）、Containerfile 拷的文件名；`tools/tests/test_license_inventory.py` 会核对这里写的摘要和文件对得上。
