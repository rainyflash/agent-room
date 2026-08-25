# 任务 33 验证记录：无障碍与低性能降级

## 1. 结论

任务 33 已完成。Pixi 只承担视觉投影，不再偷偷生成第二套可聚焦树；同一个场景投影映射为 DOM `listbox`/`option` 语义。方向键只移动活动项，`Enter` 或空格才打开详情，详情关闭后焦点回到场景；当前 Agent 的名称和文字状态通过独立 `aria-live` 区域播报。

首次自动扫描确实发现 `listbox` 内混入 live-region 的关键级 ARIA 违规。实现没有屏蔽 axe 规则，而是把视觉层、选项层和播报层物理拆开，确保 `listbox` 只拥有允许的选项子树。

## 2. 完整降级路径

- 宽度小于 720px：强制完整列表，关闭 Pixi 与悬浮信号坞。
- `forced-colors: active`：强制完整列表，并使用系统色边框与焦点轮廓。
- `deviceMemory <= 2`，或内存信息不可用且 `hardwareConcurrency <= 2`：强制完整列表。
- Pixi/WebGL 初始化失败：显示明确故障状态、完整列表和可重试操作。
- `prefers-reduced-motion: reduce`：关闭持续脉冲、滚动和弹簧位移动画；工作状态继续以文字、形状与颜色共同表达。

能力判断位于纯领域函数，React Hook 只读取媒体查询和设备提示。GPU 失败由场景适配器上抛，不把平台判断塞进 UI 组件。

## 3. 键盘与辅助技术语义

- 场景使用 `aria-activedescendant` 管理 200 个 Agent 的单一 Tab 停靠点，避免产生 200 个无意义 Tab 步骤。
- 上下左右根据画布坐标选择空间方向上的最近 Agent；活动项与已经打开的详情分离。
- 列表模式支持 `ArrowUp`、`ArrowDown`、`Home` 和 `End`，移动焦点但不误触打开。
- 详情关闭按钮自动获得焦点；关闭后恢复到场景或列表中原来的 Agent。
- Canvas 标记为纯视觉，Chromium 辅助树可读到房间场景、200 个 option、活动项及文字状态，未出现重复 Pixi 节点。

实现与验证依据 [W3C WCAG 2.2](https://www.w3.org/TR/WCAG22/)、[W3C Reflow 理解文档](https://www.w3.org/WAI/WCAG22/Understanding/reflow.html) 和 [axe-core Playwright 官方集成](https://github.com/dequelabs/axe-core-npm/tree/develop/packages/playwright)。

## 4. 自动验证

```text
pnpm check
  Prettier、ESLint、UI 文案扫描、TypeScript、协议生成物通过
  64 个 Vitest 文件、234 个测试通过

pnpm --filter @agent-room/web test:browser -- accessibility.e2e.ts lobby-scene.e2e.ts
  8/8 通过

pnpm build
  协议包与 Web/PWA 生产构建通过
```

浏览器验收覆盖：WCAG 2.2 A/AA axe 扫描、纯键盘方向导航、详情焦点闭环、200 Agent 语义树、手机列表、低动态、高对比、GPU/Canvas 故障、等效 200% 放大视口、无横向溢出和 Agent 详情核心任务。

## 5. 实际应用内浏览器走查

应用内 Chromium 在 `http://127.0.0.1:5173/e2e/fixtures/lobby-scene.html` 人工执行：

- 辅助树暴露中文 `listbox` 与 200 个有名称和文字状态的 `option`。
- 场景获得焦点后按 `ArrowRight`，`aria-activedescendant` 从当前项移动到 Agent 023，详情没有误开。
- 按 `Enter` 后只出现一个详情面板，焦点自动进入“关闭 Agent 详情”。
- 页面包含一个 Canvas 视觉层、一个详情层，`scrollWidth <= clientWidth`，无横向溢出。
- 控制台没有运行时 warning 或 error；仅有 Vite 连接日志与 React 开发提示。

该走查验证的是 Chromium 实际辅助功能树和键盘焦点路径，不伪称已经听取特定版本 NVDA/VoiceOver 的语音输出；封闭测试包仍会在任务 36 的 Windows/macOS 设备矩阵中记录具体辅助技术版本。

## 6. 提交

- `a1266f6`：建立场景 DOM 语义、方向键导航、焦点恢复、能力降级与 WCAG 浏览器门禁。

## 7. 下一步

下一项是任务 34：把发送队列、退避、同步缺口、未知提交对账、孤儿清理与各依赖故障的真实降级状态做成可故障注入的可靠性边界。
