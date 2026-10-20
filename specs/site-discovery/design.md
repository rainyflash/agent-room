# 让网站被搜到、被 AI 读懂：设计

维护者 2026-10-19 问能不能给网站做 SEO 和 GEO（让搜索引擎和 AI 助手找得到、讲得对）。当天定了：下面第 1 到第 4 件都做，MCP 目录也做（推翻 10-17 “不提交 MCP 目录”），Google Search Console 由编码 Agent 一步一步教维护者配。

## 背景

查于 2026-10-19。

- **裸域只做跳转。** `agentroom.chat` 除了 Matrix 的 `.well-known` 和 `/agents.md`、`/agents.txt`，别的路径一律 302 到 `app.agentroom.chat`（`render.py` 的 `_caddyfile`）。推广里写的都是 agentroom.chat，搜索引擎看到的却是“临时跳走”，名声记在哪个地址上说不准。
- **首页没有字。** `app.agentroom.chat/` 是单页应用，HTML 里只有一个空的 `<div id="root">`，标题只有“Agent Room”。Google 会执行脚本再看，Bing（ChatGPT 搜索靠它）、Perplexity 和各家 AI 爬虫大多不执行，看到的是空页面。
- **每个路径都回 200。** Caddy 的兜底把任何路径都当成应用（`try_files {path} /index.html`），`/asdf` 也回 200 和同一份空壳。搜索引擎管这叫“软 404”，会降低对整个站的信任。
- **没有站点地图，也没有给 AI 看的入口。** `robots.txt` 只挡了 `/watch` 和 `/join`。给 Agent 读的接入说明（`/agents.txt`）已经有了，但没有 `llms.txt` 指向它。
- **没有能被搜到的文章。** 人搜的是“怎么让 Claude Code 和 Codex 对话”“几个 AI Agent 一起干活”，站上没有一页正面回答这些问题。同名、近名的项目不少（agent-room.com、agentsroom.dev 等），光靠名字搜不到我们。
- **MCP 目录上没有我们。** 远程 MCP（`https://api.agentroom.chat/mcp`）的说明已经是英文，不登录就能列工具，但官方 MCP Registry 和各家目录都没收录。

## 怎么改

### 1. 两个域名分工：agentroom.chat 是网站，app 子域是应用

- `agentroom.chat` 改成一个**静态网站**：首页、中文首页、几篇指南、站点地图、`robots.txt`、`llms.txt`、404 页。纯 HTML 和 CSS，不需要脚本就能读全，爬虫、AI 读取器和慢网络都一样。
- `app.agentroom.chat` 照旧是应用（登录、房间、“我的 Agent”、围观页、隐私说明）。整个子域加 `X-Robots-Tag: noindex`，不进搜索结果，搜索的名声都记在 agentroom.chat 上。应用里的首页照旧（桌面端、PWA 都用它），只是不再和网站抢排名。
- 以前发出去的链接不能断：裸域上应用自己的路径（`/watch`、`/join`、`/lobby/...`、`/connect`、`/privacy`、`/guide` 等，和应用路由一份清单）照旧 302 到 app 子域，带上原来的路径和查询；`/agents.md`、`/agents.txt` 和 Matrix 的 `.well-known` 不变。
- 网站上的按钮都指向应用：“进入房间”到 `/connect`，“看看公共大厅”到 `/watch`，隐私说明到应用的 `/privacy`。下载给两个按钮（Windows、Mac），地址和应用首页同一套构建变量，静态页认不出访客的系统。
- 自建的服务器默认还是跳转（和现在一样是 302），部署配置里设了才出网站：`deployment.json` 的 `public` 里加 `site: "redirect" | "static"`，缺省 `redirect`。网站里写的是 agentroom.chat 这个部署的事，别人的服务器不该挂。生产在发版时把它改成 `static`（改之前留一份 `deployment.json.before-alpha80`）。

### 2. 网站怎么生成

- 生成器在 `apps/web/build/site/`（模板、页面、网站自己的文字），静态文件和内容在 `apps/web/site/`：
  - 一份样式。颜色、字体、圆角的取值和 `packages/ui-system` 一样（测试卡着），看起来和应用是一家。
  - 首页的标题、简介和三个用法直接取应用首页那份文案（`features/landing/i18n/landing-resources.ts`），网站和应用首页说的是同一句话，改一处两边都变。
  - 首屏的房间插图 `room.png` 和分享预览图同一个画法，由 `e2e/social-preview.e2e.ts` 截出来（`AGENT_ROOM_WRITE_SITE_ART=1` 时写进去）；两句对话气泡由网站用 HTML 叠上，中英文各写各的。
  - 指南写成 Markdown，中英文各一份（`guides/<slug>.en.md`、`guides/<slug>.zh-CN.md`），开头写标题、简介、更新日期。
- 网页构建（`pnpm --filter @agent-room/web build`）最后一步把它们生成到 `dist-site/`。网关镜像多拷一份到 `/srv/site`（`Containerfile.web`），Caddy 的裸域从那里出文件。不进应用的 `dist/`，PWA 的离线缓存不会带上它。
- 网站的地址写死成 `https://agentroom.chat`，应用写死成 `https://app.agentroom.chat`：网站上写的本来就是这个部署的事，自建默认也不出网站。下载地址用应用首页同一套构建变量，没有时指向 GitHub 上最新的发行。只有正式构建（`vite build`）生成网站，桌面端和验收用的构建不生成。
- Markdown 转 HTML 用一个小库（`marked`）。加了依赖就在同一个 PR 里重新生成许可证清单。

### 3. 每一页都要有的

- `<title>` 和 `meta description` 各写各的，不全站一句；`<html lang>` 写对。
- `canonical` 指向自己的完整地址（`https://agentroom.chat/...`，结尾带斜杠）。
- 中英文互相标 `hreflang`（`en`、`zh-CN`），`x-default` 指英文页。英文在 `/`，中文在 `/zh/`。
- 分享预览（`og:`）用同一张 `social-preview.png`，网站自己带一份。
- 首页带 JSON-LD：
  - `WebSite`：名字和 `alternateName`（agentroom.chat），Google 用它决定结果里显示的站名；
  - `SoftwareApplication`：类别、Windows、macOS 和网页、免费、版本号、GitHub 仓库（`sameAs`）。
- 指南带 `TechArticle` 的 JSON-LD（标题、更新日期、语言）。
- 不放统计、广告和第三方脚本、字体。裸域加一套严格的内容安全策略：只许自己的样式、图片和脚本。JSON-LD 不执行，不受影响。
- 英文首页有一小段脚本（单独的文件，不内联）：浏览器首选中文、又没在网站上手动选过语言时，跳到 `/zh/`。爬虫的首选语言是英文，不受影响。页面上的语言链接记下手动选过的语言，下次不再跳。

### 4. 站点地图、robots、llms.txt、404

- `sitemap.xml` 列出网站上每一页，带 `lastmod`（取指南的更新日期，首页取构建时间）和中英文互指。
- 裸域的 `robots.txt`：全部允许，写上 `Sitemap: https://agentroom.chat/sitemap.xml`。app 子域的 `robots.txt` 照旧挡 `/watch`、`/join`。
- `llms.txt` 照 llmstxt.org 的格式写：一句话说 Agent Room 是什么；接入说明（`/agents.txt`）和远程 MCP 地址；每篇指南；GitHub、自建、隐私说明。给 AI 助手一个入口，先读这份再决定读哪页。
- 裸域上不存在的路径回真的 404，页面上给回首页、指南和进入应用的链接。
- app 子域：不在应用路由清单里的路径照样给出应用的空壳（应用自己显示“找不到页面”），但状态码改成 404。浏览器和应用不受影响，只是爬虫知道这页不存在。清单哪天和路由对不上，坏处也只是状态码不对，页面照常打开。

应用路由清单写在 `render.py` 一处，第 1 节的跳转和这里的 404 共用。`tools/tests` 加一条测试：从 `apps/web/src/app/router.tsx` 读出所有 `path:`，和清单对一遍，新加路由忘了加进清单就红。

### 5. 指南

第一批四篇，中英文各一份，回答人真的会搜的问题。和 README 一样可以点名 Claude Code、Codex、ChatGPT；产品界面和提示词照旧不点名。

| slug | 中文标题 | 英文标题 |
| --- | --- | --- |
| `claude-code-codex-one-room` | 让 Claude Code 和 Codex 在一个房间里对话 | Let Claude Code and Codex talk in one room |
| `agents-working-together` | 几个 AI Agent 怎么在一个房间里分工 | How several AI agents work together in one room |
| `chatgpt-with-coding-agents` | 把网页版 ChatGPT 请进来，和电脑上的 Agent 一起聊 | Bring ChatGPT into a room with your coding agents |
| `about-the-name` | Agent Room（agentroom.chat）和名字相近的项目 | Agent Room (agentroom.chat) and similarly named projects |

- 每篇都是真实做过的步骤，截图用现成的素材，不编效果。说到加密照 README 的“谁能读到什么”，别写“服务器也读不到”。
- 第四篇只说我们是什么、和那几个项目没有关系，不评价别人的产品。
- 首页和应用的“使用指南”页底下加一行指南链接。
- 正文由编码 Agent 起草，PR 里给维护者看；它们随下一版上线，上线前维护者可以改。

### 6. MCP 目录

仓库里准备好要用的东西，每个目录提交之前先问维护者。要登录的由维护者自己登录。

- **官方 MCP Registry：** 仓库根目录加 `server.json`（2025-12-11 版的 schema）：
  - 名字 `io.github.rainyflash/agent-room`，用 GitHub 证明是我们的。维护者在浏览器里批准一次 GitHub 设备码就能发，不用碰任何密钥。想要和域名一致的 `chat.agentroom/agent-room`，就得在 DNS 上加一条 TXT、在本机保管一把私钥，先不用。
  - 只写 `remotes`（`streamable-http`，`https://api.agentroom.chat/mcp`），不写安装包，加上网站、仓库和图标。
  - 版本跟着发布版本走，`bump_release_version.py` 一起改，有测试卡着。新版本不自动发布到 Registry，工具变了再发。
  - 发布用官方的 `mcp-publisher`。下载它要维护者点头（说清文件名、来源、大小）。
- **Glama：** 根目录加 `glama.json`（维护者写 `rainyflash`），认领自动收录的条目。
- **其他目录：** 这些要么从官方 Registry 自动同步，要么要账号在网页上提交，一个一个问过再做。
  - 网页上提交：Smithery（填远程地址，它自己列工具）、mcp.so；
  - 给 awesome-mcp-servers 开 PR，用维护者的 GitHub 账号；
  - PulseMCP 从官方 Registry 同步。
- **列表文案：** 写进 `docs/mcp-directories.md`：一句话、一段说明、工具列表、怎么认证（工具参数或 `Authorization: Bearer`）、隐私说明地址，外加每个目录的提交记录（哪天、谁、链接）。各目录的字数限制不一样，按最短的写一版。
- Claude 和 ChatGPT 的应用目录要求 OAuth 登录，我们的远程 MCP 是令牌，这次不做。

### 7. 上线以后

- 维护者在 Google Search Console 加**网域资源** `agentroom.chat`（在 Vercel 的 DNS 里加一条 TXT 验证），覆盖裸域和所有子域。网站上线后提交 `https://agentroom.chat/sitemap.xml`，对首页、`/zh/` 和四篇指南用“网址检查”请求编入索引。
- Bing Webmaster Tools 从 Search Console 一键导入（ChatGPT 搜索用 Bing 的索引）。
- 上线一周、四周各看一次：收录了几页、搜什么词进来、有没有抓取报错，数字记在“状态”一节。

## 不做

- 不把应用挪到裸域：OIDC 回调、CSP、Cookie、桌面端和 Synapse 的 SSO 白名单都认 app 子域，挪一次牵扯太多，换来的只是地址短一点。
- 不在单页应用里做服务端渲染或预渲染：网站和应用分开以后，要被搜到的页面都是静态的，应用不用为爬虫改。
- 不做统计。看数用 Search Console 和 Bing 自己的报表。
- 不批量造内容页。指南只写真实做过的事，宁少勿多。

## 交付

每步一个 PR，按这个顺序：

1. 设计（这份）。
2. 网站生成器：模板、样式、中英文首页、`sitemap.xml`、`robots.txt`、`llms.txt`、404 页，网关镜像带上 `/srv/site`。Caddy 不动，生产还是跳转。
3. 四篇指南，中英文。首页和应用的“使用指南”页加指南链接。
4. Caddy：`public.site`、裸域出网站（真的 404、应用路径照旧跳）、app 子域 `noindex` 和 404 状态码、路由清单测试。它改了 `tools/prodops/render.py`，发版要维护者批准设备码。
5. MCP 目录：`server.json`、`glama.json`、`docs/mcp-directories.md`，版本号脚本跟着改。
6. CLAUDE.md：推广一节里“不提交 MCP 目录”“`sitemap.xml` 不做”两条改掉，进度记一笔。

第 2 到第 4 步要在同一个版本里发布：第 4 步让 Caddy 从 `/srv/site` 出文件，镜像里得已经有它。发版时把生产的 `public.site` 改成 `static`。上线以后才提交 sitemap 和各个 MCP 目录。

## 状态

- 2026-10-20 起草，设计 #553。
- 第 2 步 #554：网站生成器。和设计不一样的两处：
  - 网站地址没走构建参数，写死成 agentroom.chat。原因是正式镜像的构建参数写在 `.github/workflows` 里，改它就得改工作流；网站上写的本来就是这个部署的事。
  - 插图没用 SVG：场景里的机器人是一张 1.35 MB 的精灵图、加载完才画，静态渲染出来只有灰色圆圈。改成和分享预览图一样截一张 PNG（85 KB）。
- 第 3 步 #555：四篇指南，中英文各一份，放在 `apps/web/site/guides/`，front matter 多一个 `order` 定先后；每篇都要有两种语言，少一份构建就失败。Markdown 用 `marked` 18.1.0 转。应用首页那行链接和“使用指南”页的“更多”里各加一个指向网站指南的链接（中文界面链到 `/zh/guides/`）。比较页只写名字和域名、说明和我们没关系，不描述别人的产品。
- 第 4 步 #556：Caddy。开关放在部署配置的 `public.site`，没叫 `publicSite`，和别的域名设置放在一起。
  - 裸域出网站时：带内容摘要的 `/_site/` 文件长缓存，404 不带缓存；`/zh` 由 Caddy 补斜杠；`/index.html` 照样能打开，靠 `canonical` 指回 `/`。
  - 网页域名上，不在应用路由里的路径给应用壳、状态码 404；缺的 `/assets/` 文件只回 404。
  - 用本机的 `caddy:2.11.4-alpine`（和生产同一版）加上构建出的网页和网站实跑过两种模式，状态码、跳转和缓存头都对。
