# 任务 32 验证记录：中英文与本地化边界

## 1. 结论

任务 32 已完成。语言不是组件各自维护的布尔开关，而是三个明确层次：系统语言解析、Matrix 账户偏好、设备临时覆盖。首次运行跟随系统；不支持的系统语言回退英文；账户偏好跨设备同步；设备覆盖只写当前设备，不污染账户事实，切换无需重启。

所有生产 TSX 可见文案都受自动扫描约束。i18next 通过声明合并获得英文主目录的键类型；中英目录以忽略 CLDR 复数后缀的基础键为单位校验完整性，并校验 `{{placeholder}}` 集合一致。新增硬编码文案、缺失基础键或插值漂移都会在 `pnpm check` 中失败。

## 2. 语言与格式化

- 支持 `en` 与 `zh-CN`；`zh`、`zh-Hans` 和区域变体解析为简体中文，未知语言解析为英文。
- 语言控件区分“账户默认”和“仅此设备”；选择账户项会清除临时覆盖，选择设备项不写 Matrix 账户数据。
- 启动 i18n 失败时的最后一道错误页也从同一目录按系统语言读取，不再硬编码英文。
- 日期、数字、相对时间和字节单位经共享 `Intl.DateTimeFormat`、`Intl.NumberFormat` 与 `Intl.RelativeTimeFormat` 格式化；复数继续由 i18next 的 `Intl.PluralRules` 路径选择。

## 3. 显式机器翻译

正文必须先完成票据、下载、长度、媒体类型与 SHA-256 校验。只有已经显示的文本正文才出现可选翻译动作；预览阶段和正文打开阶段都不会调用翻译器。

用户点击“Translate explicitly / 明确翻译”后，适配器才探测浏览器 `Translator`、检查语言对、按需创建本地翻译器并执行翻译。译文显示在独立区域并永久标记为“Machine translation / 机器翻译”，原文仍在上方保持可见，handoff 继续引用已验证原文，不引用机器译文。

实现依据 Chrome 官方 [Translator API 文档](https://developer.chrome.com/docs/ai/translator-api)：该 API 需要能力探测，语言包可能在首次明确使用时下载，并通过 `availability()`、`create()`、`translate()` 工作。Chrome 官方状态页说明 Translator API 自 Chrome 138 桌面稳定版可用。其他 WebView 或不支持的语言对会返回明确的 `unavailable`，不会改接不透明云翻译服务。

## 4. 自动验证

```text
pnpm check
  Prettier、ESLint、UI 文案扫描、TypeScript、协议生成物通过
  63 个 Vitest 文件、228 个测试通过

pnpm --filter @agent-room/web test:browser -- i18n.e2e.ts
  2/2 通过

pnpm build
  协议包与 Web/PWA 生产构建通过
```

单元与组件测试覆盖：系统语言解析、未知值回退、设备覆盖不改账户偏好、目录基础键与占位符漂移、`Intl` 数字/相对时间/字节格式、浏览器翻译不可用、显式点击前零翻译调用、机器来源标记与原文保留。

## 5. 真实浏览器证据

应用内 Chromium 在 `http://127.0.0.1:5173/connect` 验证：

- 页面标题为 `Agent Room`，连接页有完整语义树，没有框架错误层；控制台零警告和零错误。
- 从账户英文切到设备简体中文后，`html[lang]` 立即从 `en` 变成 `zh-CN`，不刷新页面。
- 1265px 视口的 `body.scrollWidth` 与 `clientWidth` 都为 1265，无横向溢出。
- 自动化 Chromium 将所有长度大于八个字符的英文消息扩大到约两倍，在 1280px 与 390px 两个视口均无横向溢出，主操作仍可见。
- 测试运行时删除一个中文消息分支后，界面回退到英文内容，未显示裸消息键。

## 6. 提交

- `384e105`：建立类型化中英目录、设备语言覆盖、共享 `Intl` 格式化、显式本地机器翻译与浏览器验收。

## 7. 下一步

下一项是任务 33：把 Pixi 场景的 DOM 语义映射、方向键导航、焦点恢复、`aria-live`、reduced-motion、无 Canvas 列表降级、200% 缩放与高对比模式做成可自动验收的无障碍边界。
