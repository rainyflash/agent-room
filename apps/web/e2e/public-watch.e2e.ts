import AxeBuilder from '@axe-core/playwright';
import { expect, test, type Page } from '@playwright/test';

import { collectPageFailures, expectNoHorizontalOverflow } from './support/page-assertions';

// 不登录看公共大厅（specs/public-lobby-watch/design.md 第 2 步）。
const fixturePath = '/e2e/fixtures/public-watch.html';
const apiOrigin = 'https://api.agent-room.localhost:18443';
const catalogId = '0198b601-77a2-7f41-b4f4-940f291951b8';
const wcagTags = ['wcag2a', 'wcag2aa', 'wcag21a', 'wcag21aa', 'wcag22aa'] as const;

test('围观页：在线的 Agent 站在场景里，对话只能看，点人物跳到他最近说的话', async ({
  page,
}, testInfo) => {
  const failures = collectPageFailures(page);
  const outgoing = collectOutgoingRequests(page);
  await page.setViewportSize({ height: 920, width: 1_440 });
  await page.goto(fixturePath);

  await expect(page.getByRole('heading', { level: 1, name: 'Agent Room Global' })).toBeVisible();
  await expect(page.getByText('Watching · Public lobby')).toBeVisible();
  // 下线的 Quill 不在场景里；说过话的 Lin 站在一边。
  const scene = page.getByRole('listbox', { name: 'Interactive Agent room scene' });
  await expect(scene.getByRole('option')).toHaveCount(4);
  await expect(page.getByRole('button', { name: 'Room participant: Lin' })).toBeVisible();
  const log = page.getByRole('log', { name: 'Chat' });
  await expect(log.getByRole('article')).toHaveCount(8);
  await expect(log.getByText('A message that isn’t shown publicly')).toBeVisible();
  await expect(log.getByText('Shared a file')).toBeVisible();
  await expect(log.getByText('Network agent').first()).toBeVisible();
  await expect(page.getByRole('textbox')).toHaveCount(0);
  await expect(page.getByRole('button', { name: 'Log in' })).toBeVisible();
  await expect(page.getByRole('button', { name: 'Create account' })).toBeVisible();
  // 最近半分钟里说过话的冒气泡：Ada 和 Hermes，40 秒前的 Sol 不冒。
  const bubbles = page.getByRole('group', { name: 'Recent room messages' }).getByRole('button');
  await expect(bubbles).toHaveCount(2);
  await expectNoHorizontalOverflow(page);
  await expectAccessible(page);
  await page.screenshot({
    animations: 'disabled',
    path: testInfo.outputPath('public-watch-desktop.png'),
  });

  // 关掉对话再点 Lin：对话打开，跳到 Lin 最近说的那句（那句不公开）。
  await page.getByRole('button', { name: 'Return to the room' }).click();
  await expect(log).toBeHidden();
  await page.getByRole('button', { name: 'Room participant: Lin' }).click();
  await expect(log).toBeVisible();
  const linLatest = log.locator('[data-message-key="m04"]');
  await expect(linLatest).toBeFocused();
  // 滚开以后再点一次 Lin，还会滚回那一句。
  await log.evaluate((element) => {
    element.scrollTop = element.scrollHeight;
  });
  await expect(linLatest).not.toBeInViewport();
  await page.getByRole('button', { name: 'Room participant: Lin' }).click();
  await expect(linLatest).toBeInViewport();

  expect(failures).toEqual([]);
  expect(outgoing).toEqual([]);
});

test('围观页：接入 Agent 给一段现成的话，进去说话到房间页', async ({ page }) => {
  const failures = collectPageFailures(page);
  const outgoing = collectOutgoingRequests(page);
  await page.setViewportSize({ height: 920, width: 1_440 });
  await page.goto(fixturePath);

  await page
    .getByRole('navigation', { name: 'Room interactions' })
    .getByRole('button', { name: 'Bring an agent' })
    .click();
  const dialog = page.getByRole('dialog', { name: 'Bring an agent' });
  await expect(dialog).toBeVisible();
  // 默认大厅：话里不写房间名，Agent 读了说明进的就是默认大厅。
  await expect(dialog).toContainText('/agents.txt');
  await expect(dialog).not.toContainText('Agent Room Global');
  await expectAccessible(page);
  await dialog.getByRole('button', { name: 'Close' }).click();
  await expect(dialog).toBeHidden();

  await page.getByRole('link', { name: 'Join the conversation' }).click();
  await expect(page.getByTestId('lobby-route-reached')).toBeAttached();
  expect(failures).toEqual([]);
  expect(outgoing).toEqual([]);
});

test('围观页：别的公共大厅，登录了，接入 Agent 的话里写着这个大厅', async ({ page }) => {
  const failures = collectPageFailures(page);
  await page.setViewportSize({ height: 920, width: 1_440 });
  await page.goto(`${fixturePath}?signed-in&lobby=night-owls`);

  await expect(page.getByRole('heading', { level: 1, name: 'Night Owls' })).toBeVisible();
  await expect(page.getByRole('button', { name: 'Log in' })).toHaveCount(0);
  await expect(page.getByRole('link', { name: 'Join the conversation' })).toHaveCount(2);
  await page
    .getByRole('navigation', { name: 'Room interactions' })
    .getByRole('button', { name: 'Bring an agent' })
    .click();
  await expect(page.getByRole('dialog', { name: 'Bring an agent' })).toContainText('Night Owls');
  expect(failures).toEqual([]);
});

test('围观页：390px 先看对话，关掉看场景，不横向溢出', async ({ page }, testInfo) => {
  const failures = collectPageFailures(page);
  const outgoing = collectOutgoingRequests(page);
  await page.setViewportSize({ height: 844, width: 390 });
  await page.goto(fixturePath);

  const log = page.getByRole('log', { name: 'Chat' });
  await expect(log.getByRole('article')).toHaveCount(8);
  await expect(page.getByRole('link', { name: 'Join the conversation' })).toBeVisible();
  await expectNoHorizontalOverflow(page);
  await expectAccessible(page);
  await page.screenshot({
    animations: 'disabled',
    path: testInfo.outputPath('public-watch-mobile.png'),
  });

  await page.getByRole('button', { name: 'Return to the room' }).click();
  await expect(log).toBeHidden();
  const scene = page.getByRole('listbox', { name: 'Interactive Agent room scene' });
  await expect(scene.getByRole('option')).toHaveCount(4);
  await expectNoHorizontalOverflow(page);
  await page.screenshot({
    animations: 'disabled',
    path: testInfo.outputPath('public-watch-mobile-scene.png'),
  });
  await page
    .getByRole('navigation', { name: 'Room interactions' })
    .getByRole('button', { name: 'Chat' })
    .click();
  await expect(log).toBeVisible();
  expect(failures).toEqual([]);
  expect(outgoing).toEqual([]);
});

for (const { name, path, width, locale } of [
  { name: 'empty-desktop', path: '?empty', width: 1_440, locale: 'en-US' },
  { name: 'empty-mobile', path: '?empty', width: 390, locale: 'en-US' },
  { name: 'long-desktop', path: '?long&stale', width: 1_440, locale: 'en-US' },
  // 浏览器是中文的访客：没登录，跟着浏览器的语言走。
  { name: 'long-mobile-zh', path: '?long&stale', width: 390, locale: 'zh-CN' },
]) {
  test.describe(name, () => {
    test.use({ locale });

    test(`围观页 ${name}：没人、人多、读不到时都不走样`, async ({ page }, testInfo) => {
      const failures = collectPageFailures(page);
      await page.setViewportSize({ height: width === 390 ? 844 : 920, width });
      await page.goto(`${fixturePath}${path}`);

      await expect(page.getByRole('heading', { level: 1 })).toBeVisible();
      if (path === '?empty') {
        await expect(
          page.getByRole('heading', { name: 'No one has said anything yet' }),
        ).toBeVisible();
        await expect(page.getByText('No agents are here right now')).toBeAttached();
      } else {
        await expect(page.getByRole('log').getByRole('article')).toHaveCount(50);
        const stale = locale === 'zh-CN' ? '正在重试' : 'Retrying';
        await expect(page.getByRole('status').filter({ hasText: stale })).toBeVisible();
      }
      await expectNoHorizontalOverflow(page);
      await expectAccessible(page);
      expect(failures).toEqual([]);
      await page.screenshot({
        animations: 'disabled',
        path: testInfo.outputPath(`public-watch-${name}.png`),
      });
    });
  });
}

test('首页“看看公共大厅”不登录就能打开，快照每 5 秒刷新，读不到时接着显示上一份', async ({
  baseURL,
  page,
}) => {
  test.setTimeout(45_000);
  const failures = collectPageFailures(page);
  const server = await mockServer(page, baseURL, {
    default: [
      { status: 200, body: snapshot(['m1']) },
      { status: 503, body: problem('public_watch.unavailable', true) },
      { status: 200, body: snapshot(['m1', 'm2']) },
    ],
  });
  await page.goto('/');
  await page.getByRole('link', { name: 'Watch the public lobby' }).click();

  await expect(page).toHaveURL(/\/watch$/u);
  await expect(page.getByRole('heading', { level: 1, name: 'Agent Room Global' })).toBeVisible();
  await expect(page.locator('meta[name="robots"]')).toHaveAttribute('content', 'noindex');
  const log = page.getByRole('log', { name: 'Chat' });
  await expect(log.getByRole('article')).toHaveCount(1);
  // 登录和注册完回到这个大厅。
  await expect(page.getByRole('button', { name: 'Log in' })).toBeVisible();

  const stale = page.getByRole('status').filter({ hasText: 'Retrying' });
  await expect(stale).toBeVisible({ timeout: 12_000 });
  await expect(log.getByRole('article')).toHaveCount(1);
  await expect(log.getByText('Second message')).toBeVisible({ timeout: 12_000 });
  await expect(stale).toBeHidden();
  expect(server.watchCalls.get('default')).toBeGreaterThanOrEqual(3);
  expect(server.unexpected).toEqual([]);
  expect(unexpected(failures)).toEqual([]);
});

test('登录按钮带着这个大厅去登录，登录完直接进去说话', async ({ baseURL, page }) => {
  await mockServer(page, baseURL, { default: [{ status: 200, body: snapshot(['m1']) }] });
  await page.route(`${apiOrigin}/auth/oidc/start?**`, async (route) => {
    await route.fulfill({
      body: '<!doctype html><title>Sign-in boundary</title>',
      contentType: 'text/html',
      status: 200,
    });
  });
  await page.goto('/watch');

  await page.getByRole('button', { name: 'Log in' }).click();
  await page.waitForURL(`${apiOrigin}/auth/oidc/start?**`);

  const target = new URL(page.url());
  expect(target.origin).toBe(apiOrigin);
  expect(target.pathname).toBe('/auth/oidc/start');
  expect(target.searchParams.get('returnTo')).toBe(`/lobby/${catalogId}`);
  expect(target.searchParams.get('intent')).toBe('sign-in');
});

test('登录了的人在围观页直接进去说话', async ({ baseURL, page }) => {
  const failures = collectPageFailures(page);
  const server = await mockServer(
    page,
    baseURL,
    { default: [{ status: 200, body: snapshot(['m1']) }] },
    {
      authenticatedAtUnixMs: Date.now() - 60_000,
      displayName: 'Returning member',
      expiresAtUnixMs: Date.now() + 3_600_000,
      locale: 'en',
      matrixUserId: '@human:matrix.test',
      principalId: '0198b601-77a1-7bb8-83eb-a8fe68c97e42',
      recentlyAuthenticated: false,
    },
  );
  await page.goto('/watch');

  await expect(page.getByRole('heading', { level: 1, name: 'Agent Room Global' })).toBeVisible();
  await expect(page.getByRole('button', { name: 'Log in' })).toHaveCount(0);
  const join = page.locator('.public-watch__account').getByRole('link', {
    name: 'Join the conversation',
  });
  await expect(join).toHaveAttribute('href', `/lobby/${catalogId}`);
  expect(server.unexpected).toEqual([]);
  expect(unexpected(failures)).toEqual([]);
});

test('没有这个公共大厅时说清楚，可以去看默认大厅', async ({ baseURL, page }) => {
  const failures = collectPageFailures(page);
  const server = await mockServer(page, baseURL, {
    'night-owls': [{ status: 404, body: problem('public_watch.lobby_not_found', false) }],
    default: [{ status: 200, body: snapshot(['m1']) }],
  });
  await page.goto('/watch/night-owls');

  await expect(
    page.getByRole('heading', { name: 'This public lobby doesn’t exist' }),
  ).toBeVisible();
  await expect(page.locator('meta[name="robots"]')).toHaveAttribute('content', 'noindex');
  await expectNoHorizontalOverflow(page);
  await page.getByRole('link', { name: 'Watch the public lobby' }).click();
  await expect(page).toHaveURL(/\/watch$/u);
  await expect(page.getByRole('heading', { level: 1, name: 'Agent Room Global' })).toBeVisible();
  expect(server.watchCalls.get('night-owls')).toBe(1);
  expect(server.unexpected).toEqual([]);
  expect(unexpected(failures)).toEqual([]);
});

test('服务器没开放围观时不重试；暂时看不了时可以再试', async ({ baseURL, page }) => {
  const failures = collectPageFailures(page);
  const server = await mockServer(page, baseURL, {
    default: [{ status: 404, body: problem('public_watch.disabled', false) }],
  });
  await page.goto('/watch');

  await expect(
    page.getByRole('heading', { name: 'This server doesn’t show the public lobby to visitors' }),
  ).toBeVisible();
  await expect(page.getByRole('button', { name: 'Try again' })).toHaveCount(0);
  await expect(page.getByRole('link', { name: 'Home' })).toBeVisible();

  await server.answer('default', [
    { status: 503, body: problem('public_watch.unavailable', true) },
    { status: 200, body: snapshot(['m1']) },
  ]);
  await page.reload();
  await expect(
    page.getByRole('heading', { name: 'The public lobby can’t be shown right now' }),
  ).toBeVisible();
  await page.getByRole('button', { name: 'Try again' }).click();
  await expect(page.getByRole('heading', { level: 1, name: 'Agent Room Global' })).toBeVisible();
  expect(server.unexpected).toEqual([]);
  expect(unexpected(failures)).toEqual([]);
});

async function expectAccessible(page: Page): Promise<void> {
  const scan = await new AxeBuilder({ page }).withTags([...wcagTags]).analyze();
  expect(
    scan.violations.map(({ id, nodes }) => ({
      id,
      nodes: nodes.map(({ target, failureSummary }) => ({ target, failureSummary })),
    })),
  ).toEqual([]);
}

/** 夹具的服务地址都是 `.invalid`：页面要是往外发了请求（比如要登录的计时接口），这里记下来。 */
function collectOutgoingRequests(page: Page): string[] {
  const requests: string[] = [];
  page.on('request', (request) => {
    if (new URL(request.url()).hostname.endsWith('.invalid')) requests.push(request.url());
  });
  return requests;
}

type WatchAnswer = { readonly status: number; readonly body: unknown };

function snapshot(messages: readonly ('m1' | 'm2')[]) {
  const sentAt = Date.now() - 60_000;
  const texts = { m1: 'First message', m2: 'Second message' };
  return {
    schemaVersion: 1,
    lobby: { catalogId, name: 'Agent Room Global', slug: 'agent-room-global' },
    participants: [
      { key: 'pq3T0vXw1aBc', name: 'Sol', kind: 'networkAgent', online: true, status: 'idle' },
      { key: 'pZ8kLm2nOpQr', name: 'Ada', kind: 'agent', online: true, status: 'idle' },
    ],
    messages: messages.map((key, index) => ({
      key,
      author: index % 2 === 0 ? 'pq3T0vXw1aBc' : 'pZ8kLm2nOpQr',
      text: texts[key],
      truncated: false,
      withheld: false,
      attachment: false,
      edited: false,
      replyTo: null,
      sentAtUnixMs: sentAt + index * 1_000,
    })),
    updatedAtUnixMs: Date.now(),
  };
}

function problem(code: string, retryable: boolean) {
  return {
    category: retryable ? 'unavailable' : 'not_found',
    code,
    correlationId: '0198b601-77a1-7bb8-83eb-a8fe68c97e42',
    details: {},
    message: code,
    retryable,
  };
}

/**
 * 假的控制面：没登录时 `/auth/session` 回 401；每个大厅按顺序回给定的答案，最后一个一直重复。
 * 别的请求都记进 `unexpected`：围观页不该去碰要登录的接口。
 */
async function mockServer(
  page: Page,
  baseURL: string | undefined,
  lobbies: Record<string, readonly WatchAnswer[]>,
  session: object | null = null,
) {
  if (baseURL === undefined) throw new Error('Missing browser base URL');
  const headers = {
    'access-control-allow-credentials': 'true',
    'access-control-allow-origin': new URL(baseURL).origin,
    'content-type': 'application/json',
  };
  const answers = new Map(Object.entries(lobbies));
  const watchCalls = new Map<string, number>();
  const unexpectedRequests: string[] = [];
  await page.route(`${apiOrigin}/**`, async (route) => {
    const url = new URL(route.request().url());
    const watch = /^\/public-lobbies\/([^/]+)\/watch$/u.exec(url.pathname);
    if (watch?.[1] !== undefined) {
      const slug = watch[1];
      const calls = watchCalls.get(slug) ?? 0;
      watchCalls.set(slug, calls + 1);
      const queue = answers.get(slug) ?? [];
      const answer = queue[Math.min(calls, queue.length - 1)];
      if (answer === undefined) throw new Error(`没有给 ${slug} 准备回答`);
      await route.fulfill({ headers, json: answer.body, status: answer.status });
      return;
    }
    if (url.pathname === '/auth/session') {
      await route.fulfill(
        session === null
          ? { body: '{}', headers, status: 401 }
          : { headers, json: session, status: 200 },
      );
      return;
    }
    if (url.pathname === '/health/ready') {
      await route.fulfill({
        headers,
        json: {
          checkedAtUnixMs: Date.now(),
          correlationId: '0198b601-77a1-7bb8-83eb-a8fe68c97e42',
          dependencies: [],
          service: 'agent-room-control-plane',
          status: 'ready',
          version: '0.1.0',
        },
      });
      return;
    }
    unexpectedRequests.push(`${route.request().method()} ${url.pathname}`);
    await route.fulfill({ body: '{}', headers, status: 404 });
  });
  return {
    unexpected: unexpectedRequests,
    watchCalls,
    /** 换掉一个大厅接下来的回答，从头数起。 */
    answer: (slug: string, queue: readonly WatchAnswer[]) => {
      answers.set(slug, queue);
      watchCalls.set(slug, 0);
      return Promise.resolve();
    },
  };
}

/** 没登录问 `/auth/session` 是 401，没开放、没有这个大厅是 404，暂时不行是 503：浏览器都会记一条。 */
function unexpected(failures: readonly string[]): readonly string[] {
  return failures.filter((text) => !/status of (401|404|503) /u.test(text));
}
