import { expect, test } from '@playwright/test';
import { mkdir, writeFile } from 'node:fs/promises';
import path from 'node:path';

import { collectPageFailures, expectNoHorizontalOverflow } from './support/page-assertions';

const fixturePath = '/e2e/fixtures/lobby-scene.html';

test('200 个 Agent 的全幅场景、键盘导航与焦点恢复可用', async ({ page }) => {
  const failures = collectPageFailures(page);
  await page.setViewportSize({ height: 900, width: 1_440 });
  await page.goto(fixturePath);

  const scene = page.getByRole('listbox', { name: 'Interactive Agent room scene' });
  await expect(scene.locator('canvas')).toBeVisible();
  await expect(page.locator('.room-beacon')).toContainText('200 agents');
  await expect(scene.getByRole('option')).toHaveCount(200);

  await scene.focus();
  await page.keyboard.press('ArrowRight');
  await expect(page.getByRole('complementary')).toHaveCount(0);
  await expect(scene).toHaveAttribute('aria-activedescendant', /lobby-scene-.+/u);
  await page.keyboard.press('Enter');
  await expect(page.getByRole('complementary')).toBeVisible();
  await page.screenshot({
    animations: 'disabled',
    fullPage: true,
    path: '../../artifacts/browser/task-20/playwright-lobby-inspector.png',
  });
  await page.getByRole('button', { name: 'Close Agent details' }).click();
  await expect(scene).toBeFocused();
  await expect(page.getByRole('complementary')).toHaveCount(0);
  await expectNoHorizontalOverflow(page);
  expect(failures).toEqual([]);

  await page.screenshot({
    animations: 'disabled',
    fullPage: true,
    path: '../../artifacts/browser/task-20/playwright-lobby-desktop.png',
  });
});

test('200 节点场景交互保持在有界帧预算内', async ({ page }, testInfo) => {
  const developerTools = await page.context().newCDPSession(page);
  await developerTools.send('Performance.enable');
  await page.setViewportSize({ height: 900, width: 1_440 });
  await page.goto(fixturePath);
  const canvas = page.locator('.lobby-scene__canvas');
  await expect(canvas).toBeVisible();

  const sceneHost = page.locator('.lobby-scene__pixi');
  const interactionSamples: {
    readonly renderMilliseconds: number;
    readonly scheduleMilliseconds: number;
    readonly updateMilliseconds: number;
  }[] = [];
  await expect(sceneHost).toHaveAttribute('data-agent-room-render-sequence', /^\d+$/u);
  await canvas.hover();
  for (let index = 0; index < 72; index += 1) {
    const previousSequence = await canvas.evaluate((element) => {
      const host = element.closest<HTMLElement>('.lobby-scene__pixi');
      if (host === null) {
        throw new Error('大厅场景缺少性能遥测宿主。');
      }
      const sequence = Number(host.dataset.agentRoomRenderSequence ?? Number.NaN);
      if (!Number.isFinite(sequence)) {
        throw new Error('大厅渲染序列无效。');
      }
      delete host.dataset.agentRoomTestRenderCompletedAt;
      delete host.dataset.agentRoomTestWheelStartedAt;
      const observer = new MutationObserver(() => {
        const nextSequence = Number(host.dataset.agentRoomRenderSequence ?? Number.NaN);
        if (Number.isFinite(nextSequence) && nextSequence > sequence) {
          host.dataset.agentRoomTestRenderCompletedAt = String(performance.now());
          observer.disconnect();
        }
      });
      observer.observe(host, {
        attributeFilter: ['data-agent-room-render-sequence'],
        attributes: true,
      });
      element.addEventListener(
        'wheel',
        () => {
          host.dataset.agentRoomTestWheelStartedAt = String(performance.now());
        },
        { capture: true, once: true },
      );
      return sequence;
    });
    await page.mouse.wheel(0, index % 12 < 6 ? -5 : 5);
    await page.waitForFunction(
      (sequence) => {
        const host = document.querySelector<HTMLElement>('.lobby-scene__pixi');
        return (
          host?.dataset.agentRoomTestRenderCompletedAt !== undefined &&
          Number(host.dataset.agentRoomRenderSequence ?? Number.NaN) > sequence
        );
      },
      previousSequence,
      { polling: 'raf', timeout: 1_000 },
    );
    interactionSamples.push(
      await sceneHost.evaluate((host) => {
        const completedAt = Number(host.dataset.agentRoomTestRenderCompletedAt ?? Number.NaN);
        const startedAt = Number(host.dataset.agentRoomTestWheelStartedAt ?? Number.NaN);
        const renderMilliseconds = Number(host.dataset.agentRoomRenderMilliseconds ?? Number.NaN);
        const updateMilliseconds = Number(host.dataset.agentRoomUpdateMilliseconds ?? Number.NaN);
        if (
          ![completedAt, renderMilliseconds, startedAt, updateMilliseconds].every(Number.isFinite)
        ) {
          throw new Error('大厅交互没有产生完整的性能遥测。');
        }
        return {
          renderMilliseconds,
          scheduleMilliseconds: completedAt - startedAt,
          updateMilliseconds,
        };
      }),
    );
  }
  interactionSamples.splice(0, 6);

  const renderDurations = interactionSamples
    .map((sample) => sample.renderMilliseconds)
    .toSorted((left, right) => left - right);
  const scheduleDurations = interactionSamples
    .map((sample) => sample.scheduleMilliseconds)
    .toSorted((left, right) => left - right);
  const updateDurations = interactionSamples
    .map((sample) => sample.updateMilliseconds)
    .toSorted((left, right) => left - right);
  const renderMedian = percentile(renderDurations, 0.5);
  const renderP95 = percentile(renderDurations, 0.95);
  const scheduleMedian = percentile(scheduleDurations, 0.5);
  const scheduleP95 = percentile(scheduleDurations, 0.95);
  const updateMedian = percentile(updateDurations, 0.5);
  const updateP95 = percentile(updateDurations, 0.95);
  const runtimeBudget = await collectRuntimeBudget(page, developerTools);
  await testInfo.attach('frame-budget.json', {
    body: Buffer.from(
      JSON.stringify({
        renderMedianMilliseconds: renderMedian,
        renderP95Milliseconds: renderP95,
        scheduleMedianMilliseconds: scheduleMedian,
        scheduleP95Milliseconds: scheduleP95,
        updateMedianMilliseconds: updateMedian,
        updateP95Milliseconds: updateP95,
        ...runtimeBudget,
      }),
    ),
    contentType: 'application/json',
  });
  testInfo.annotations.push({
    description: `update median=${updateMedian.toFixed(2)}ms, update p95=${updateP95.toFixed(2)}ms, render median=${renderMedian.toFixed(2)}ms, render p95=${renderP95.toFixed(2)}ms, schedule median=${scheduleMedian.toFixed(2)}ms, schedule p95=${scheduleP95.toFixed(2)}ms`,
    type: 'performance',
  });
  expect(updateMedian).toBeLessThanOrEqual(22);
  expect(updateP95).toBeLessThanOrEqual(40);
  if (process.env.AGENT_ROOM_CAPACITY_REPORT === '1') {
    expect(scheduleMedian).toBeLessThanOrEqual(22);
    expect(scheduleP95).toBeLessThanOrEqual(40);
  }
  expect(runtimeBudget.textureCount).toBeLessThanOrEqual(256);
  expect(runtimeBudget.renderedNodes).toBeLessThanOrEqual(200);
  expect(runtimeBudget.messageNodes).toBeLessThanOrEqual(200);
  if (process.env.AGENT_ROOM_CAPACITY_REPORT === '1') {
    expect(runtimeBudget.javascriptHeapBytes).toBeLessThanOrEqual(256 * 1_024 * 1_024);
    expect(runtimeBudget.resourceCount).toBeLessThanOrEqual(80);
    expect(runtimeBudget.decodedResourceBytes).toBeLessThanOrEqual(12 * 1_024 * 1_024);
  }
  expect(runtimeBudget.externalImageResources).toBe(0);
  await writeCapacityReport({
    medianMilliseconds: scheduleMedian,
    p95Milliseconds: scheduleP95,
    renderMedianMilliseconds: renderMedian,
    renderP95Milliseconds: renderP95,
    updateMedianMilliseconds: updateMedian,
    updateP95Milliseconds: updateP95,
    ...runtimeBudget,
  });
});

test('手机默认使用完整列表且 reduced-motion 不保留持续动画', async ({ page }) => {
  const failures = collectPageFailures(page);
  await page.emulateMedia({ reducedMotion: 'reduce' });
  await page.setViewportSize({ height: 844, width: 390 });
  await page.goto(fixturePath);

  await expect(page.getByRole('heading', { name: 'Agent roster' })).toBeVisible();
  await expect(page.locator('.signal-dock')).toHaveCount(0);
  await expect(page.locator('.list-roster__list > li')).toHaveCount(200);
  await expect(page.locator('canvas')).toHaveCount(0);
  await expect(page.locator('.ar-status-mark--pulse')).toHaveCount(0);
  await expectNoHorizontalOverflow(page);
  expect(failures).toEqual([]);

  await page.screenshot({
    animations: 'disabled',
    fullPage: true,
    path: '../../artifacts/browser/task-20/playwright-lobby-mobile.png',
  });
});

test('WebGL 不可用时自动降级为可交互 SVG 空间视图', async ({ page }) => {
  const failures = collectPageFailures(page);
  await page.addInitScript(() => {
    Object.defineProperty(HTMLCanvasElement.prototype, 'getContext', {
      configurable: true,
      value: () => null,
    });
  });
  await page.setViewportSize({ height: 900, width: 1_440 });
  await page.goto(fixturePath);

  const scene = page.getByRole('listbox', { name: 'Interactive Agent room scene' });
  await expect(page.locator('[data-renderer="svg"]')).toBeVisible();
  await expect(scene.getByRole('option')).toHaveCount(200);
  await expect(page.getByText(/graphics surface failed/u)).toHaveCount(0);
  const activeOptionText = await scene.getByRole('option', { selected: true }).innerText();
  const activeAgentName = /^Build Agent \d{3}/u.exec(activeOptionText)?.[0];
  if (activeAgentName === undefined) {
    throw new Error('SVG 降级场景缺少有效的当前 Agent。');
  }
  await scene.focus();
  await page.keyboard.press('Enter');
  await expect(page.getByRole('complementary')).toContainText(activeAgentName);
  await page.getByRole('button', { name: 'Close Agent details' }).click();
  await expect(scene).toBeFocused();
  expect(failures).toEqual([]);
});

test('直接会话从 Agent 资料进入并保持正文按需读取', async ({ page }) => {
  const failures = collectPageFailures(page);
  await page.setViewportSize({ height: 900, width: 1_440 });
  await page.goto(fixturePath);

  await page.getByRole('button', { name: 'Open conversation with Build Agent 002' }).click();
  const conversation = page.locator('.direct-conversation');
  await expect(conversation).toBeVisible();
  await expect(conversation.getByRole('heading', { name: 'Build Agent 002' })).toBeVisible();
  await expect(
    conversation.locator('.message-signal__title').filter({ hasText: 'Protocol review ready' }),
  ).toBeVisible();

  expect(await fixtureContentReadCount(page)).toBe(0);
  await conversation
    .locator('.message-signal')
    .filter({ hasText: 'Protocol review ready' })
    .click();
  await expect(page.getByRole('heading', { name: 'Protocol review ready' })).toBeVisible();
  expect(await fixtureContentReadCount(page)).toBe(0);
  await page.getByRole('button', { name: 'Open full content' }).click();
  await expect(
    page.getByRole('heading', { exact: true, name: 'Remote task result' }),
  ).toBeVisible();
  expect(await fixtureContentReadCount(page)).toBe(1);
  await page.getByRole('button', { name: 'Close message details' }).click();

  await conversation.getByRole('button', { name: 'Block' }).click();
  await expect(conversation.getByText('You blocked this Agent')).toBeVisible();
  await conversation.getByRole('button', { name: 'Unblock' }).click();
  await expect(conversation.getByText('Delivery allowed')).toBeVisible();
  await conversation.getByRole('button', { name: 'Close direct conversation' }).click();
  await expect(conversation).toHaveCount(0);

  await page.getByRole('button', { name: 'List view' }).click();
  await page.getByRole('button', { name: /Build Agent 001/u }).click();
  await page.getByRole('button', { name: 'Message Agent' }).click();
  await expect(page.locator('.agent-inspector')).toHaveCount(0);
  await expect(
    page.locator('.direct-conversation').getByRole('heading', { name: 'Build Agent 001' }),
  ).toBeVisible();
  await expect(
    page.getByRole('button', { name: 'Open conversation with Build Agent 001' }),
  ).toBeVisible();
  await expectNoHorizontalOverflow(page);
  expect(failures).toEqual([]);

  await page.screenshot({
    animations: 'disabled',
    fullPage: true,
    path: '../../artifacts/browser/task-26/playwright-direct-session.png',
  });

  await page.setViewportSize({ height: 844, width: 390 });
  await expect(page.locator('.direct-conversation')).toBeVisible();
  await expectNoHorizontalOverflow(page);
  await page.screenshot({
    animations: 'disabled',
    fullPage: true,
    path: '../../artifacts/browser/task-26/playwright-direct-session-mobile.png',
  });
});

function percentile(values: readonly number[], ratio: number): number {
  const index = Math.min(values.length - 1, Math.floor(values.length * ratio));
  return values[index] ?? Number.POSITIVE_INFINITY;
}

async function fixtureContentReadCount(page: import('@playwright/test').Page): Promise<number> {
  return await page.evaluate(() => {
    const fixtureWindow = window as Window & {
      readonly __agentRoomFixtureContentReads?: { readonly downloads: number };
    };
    return fixtureWindow.__agentRoomFixtureContentReads?.downloads ?? -1;
  });
}

async function collectRuntimeBudget(
  page: import('@playwright/test').Page,
  developerTools: import('@playwright/test').CDPSession,
) {
  const performanceMetrics = await developerTools.send('Performance.getMetrics');
  const javascriptHeapBytes =
    performanceMetrics.metrics.find((metric) => metric.name === 'JSHeapUsedSize')?.value ??
    Number.POSITIVE_INFINITY;
  const browserMetrics = await page.evaluate(() => {
    const resourceEntries = performance.getEntriesByType('resource') as PerformanceResourceTiming[];
    const scene = document.querySelector<HTMLElement>('.lobby-scene__pixi');
    return {
      decodedResourceBytes: resourceEntries.reduce(
        (total, resource) => total + resource.decodedBodySize,
        0,
      ),
      externalImageResources: resourceEntries.filter(
        (resource) =>
          resource.initiatorType === 'img' && new URL(resource.name).origin !== location.origin,
      ).length,
      messageNodes: document.querySelectorAll('.message-signal').length,
      renderedNodes: Number(scene?.dataset.agentRoomRenderedNodes ?? Number.NaN),
      resourceCount: resourceEntries.length,
      textureCount: Number(scene?.dataset.agentRoomTextureCount ?? Number.NaN),
    };
  });
  return { javascriptHeapBytes, ...browserMetrics };
}

async function writeCapacityReport(metrics: Readonly<Record<string, number>>): Promise<void> {
  if (process.env.AGENT_ROOM_CAPACITY_REPORT !== '1') {
    return;
  }
  const revision = process.env.AGENT_ROOM_CAPACITY_REVISION;
  if (revision === undefined || revision.length < 7) {
    throw new Error('容量报告缺少 Git 修订。');
  }
  const reportPath = path.resolve('../../artifacts/capacity/web-report.json');
  await mkdir(path.dirname(reportPath), { recursive: true });
  await writeFile(
    reportPath,
    `${JSON.stringify(
      {
        schemaVersion: 1,
        scenario: 'web_200_node_budget',
        evidenceLevel: 'real_chromium_webgl',
        generatedAt: new Date().toISOString(),
        revision,
        passed: true,
        releaseGateEligible: true,
        topology: {
          browser: 'Chromium through Playwright CDP',
          nodes: 200,
          viewport: '1440x900',
        },
        metrics,
      },
      null,
      2,
    )}\n`,
    'utf8',
  );
}
