import AxeBuilder from '@axe-core/playwright';
import { expect, test, type Page } from '@playwright/test';

import { collectPageFailures, expectNoHorizontalOverflow } from './support/page-assertions';

// 发现页（specs/public-rooms/design.md“发现页”）。夹具的用法写在 src/test/explore-fixture.tsx 开头。
const fixture = '/e2e/fixtures/explore.html';

async function expectAccessible(page: Page): Promise<void> {
  const scan = await new AxeBuilder({ page })
    .include('main')
    .withTags(['wcag2a', 'wcag2aa', 'wcag21a', 'wcag21aa', 'wcag22aa'])
    .analyze();
  expect(
    scan.violations.map(({ id, nodes }) => ({ id, targets: nodes.map(({ target }) => target) })),
  ).toEqual([]);
}

function card(page: Page, name: string) {
  return page.getByRole('listitem').filter({ has: page.getByRole('heading', { name }) });
}

for (const width of [1440, 390]) {
  test(`发现页：公共大厅、公开房间的卡片、按处境写的按钮、筛选：${String(width)}`, async ({
    page,
  }, testInfo) => {
    const failures = collectPageFailures(page);
    await page.setViewportSize({ width, height: width === 390 ? 844 : 1000 });
    await page.goto(fixture);

    await expect(page.getByRole('heading', { level: 1 })).toHaveText('Explore public rooms');
    await expect(page.locator('.app-navigation__links [aria-current="page"]')).toHaveText(
      'Explore',
    );
    await expect(page.locator('meta[name="robots"]')).toHaveAttribute('content', 'noindex');

    // 公共大厅：登录了直接进。
    const lobbies = page.getByRole('region', { name: 'Public lobbies' });
    await expect(
      lobbies.getByRole('link', { name: 'Enter · Default public lobby' }),
    ).toHaveAttribute('href', '/lobby/0198b601-77a2-7f41-b4f4-940f291951b8');

    // 卡片：房主、人数、多久以前有人说话、加入方式和加不加密。
    const rust = card(page, 'Rust help desk');
    await expect(rust).toContainText('Owner: Grace');
    await expect(rust).toContainText('18 people');
    await expect(rust).toContainText('4 agents');
    await expect(rust).toContainText('Last message 7 minutes ago');
    await expect(rust).toContainText('Join directly');
    await expect(rust).toContainText('Not encrypted');
    await expect(rust.getByRole('button', { name: 'Join · Rust help desk' })).toBeVisible();

    // 按钮看你的处境。
    const reading = card(page, '周末读书会');
    await expect(reading).toContainText('Waiting for approval');
    await expect(
      reading.getByRole('button', { name: 'Withdraw request · 周末读书会' }),
    ).toBeVisible();
    await expect(
      card(page, 'Design critiques').getByRole('link', { name: 'Ask to join · Design critiques' }),
    ).toHaveAttribute('href', '/explore/Rm0003');
    const tinkering = card(page, 'Agent tinkering');
    await expect(tinkering).toContainText('Nobody has spoken yet');
    await expect(tinkering).toContainText('Encrypted');
    await expect(
      tinkering.getByRole('link', { name: 'Joined · Enter · Agent tinkering' }),
    ).toHaveAttribute('href', /\/lobby\/0198c000-/u);
    await expect(card(page, 'Board game nights')).toContainText('Invite only');
    await expect(card(page, 'Board game nights').getByRole('link')).toHaveCount(1);
    await expect(
      card(page, 'Paper club').getByRole('button', { name: 'Accept invitation · Paper club' }),
    ).toBeVisible();
    await expect(card(page, 'Old arguments').getByRole('link')).toHaveCount(1);

    await expectNoHorizontalOverflow(page);
    await expectAccessible(page);
    await page.screenshot({
      fullPage: true,
      path: testInfo.outputPath(`explore-${String(width)}.png`),
    });

    // 名字的链接铺满卡片：点在人数那一行上，接住点击的是它，打开详情。
    await rust.locator('.explore-room__facts').click({ force: true });
    await expect(page.locator('body')).toHaveAttribute('data-fixture-opened', '/explore/Rm0001');
    await expect(page.getByRole('dialog', { name: 'Rust help desk' })).toBeVisible();
    await page.keyboard.press('Escape');
    await expect(page.getByRole('dialog')).toHaveCount(0);
    await expect(page.locator('body')).toHaveAttribute('data-fixture-opened', '/explore');

    // 筛选和排序。
    await page.getByRole('button', { name: 'Ask to join' }).click();
    await expect(page.getByRole('button', { name: 'Ask to join' })).toHaveAttribute(
      'aria-pressed',
      'true',
    );
    await expect(page.locator('.explore-room h3')).toHaveText(['周末读书会', 'Design critiques']);
    await page.getByLabel('Sort by').selectOption('members');
    await expect(page.locator('.explore-room h3')).toHaveText(['周末读书会', 'Design critiques']);
    await expect(page.locator('body')).toHaveAttribute(
      'data-fixture-filter',
      JSON.stringify({ q: '', quick: 'request', sort: 'members' }),
    );

    // 搜索停手才去问。
    await page.getByRole('button', { name: 'All', exact: true }).click();
    await page
      .getByRole('searchbox', { name: 'Search room names, descriptions and owners' })
      .fill('rust');
    await expect(page.locator('.explore-room h3')).toHaveText(['Rust help desk']);

    // 什么都没搜到。
    await page.getByRole('searchbox').fill('nothing like this');
    await expect(page.getByRole('status')).toContainText('No rooms found');
    await page.getByRole('button', { name: 'Open a public room' }).click();
    await expect(page.locator('body')).toHaveAttribute('data-fixture-create', 'public');
    expect(failures).toEqual([]);
  });

  test(`发现页：没登录时顶栏是登录和注册，卡片上是“登录后加入”：${String(width)}`, async ({
    page,
  }, testInfo) => {
    const failures = collectPageFailures(page);
    await page.setViewportSize({ width, height: width === 390 ? 844 : 1000 });
    await page.goto(`${fixture}?signed-out`);

    await expect(page.getByRole('navigation')).toHaveCount(0);
    await expect(
      page.getByRole('region', { name: 'Public lobbies' }).getByRole('link', {
        name: 'Watch · Default public lobby',
      }),
    ).toHaveAttribute('href', '/watch/default-public');
    const rust = card(page, 'Rust help desk');
    await rust.getByRole('button', { name: 'Sign in to join · Rust help desk' }).click();
    // 登录完回到这间房的详情，接着加入。
    await expect(page.locator('body')).toHaveAttribute(
      'data-fixture-authentication',
      'sign-in:/explore/Rm0001?join=1',
    );
    await page.getByRole('button', { name: 'Log in' }).click();
    await expect(page.locator('body')).toHaveAttribute('data-fixture-authentication', 'sign-in');
    await page.getByRole('button', { name: 'Create account' }).click();
    await expect(page.locator('body')).toHaveAttribute('data-fixture-authentication', 'register');
    // 已经在里面、只能邀请的照旧。
    await expect(card(page, 'Board game nights')).toContainText('Invite only');
    // 没搜到时不给“自己开一间”：要先登录。
    await expectNoHorizontalOverflow(page);
    await expectAccessible(page);
    await page.screenshot({
      fullPage: true,
      path: testInfo.outputPath(`explore-signed-out-${String(width)}.png`),
    });
    expect(failures).toEqual([]);
  });
}

test('发现页：没读到、被限流、还在读、没有房间', async ({ page }) => {
  const failures = collectPageFailures(page);
  await page.goto(`${fixture}?failed`);
  await expect(page.getByRole('alert')).toContainText('Couldn’t load public rooms');
  await expect(page.getByRole('alert')).toContainText('public_room.unreachable');
  await page.getByRole('button', { name: 'Try again' }).click();
  await expect(page.locator('body')).toHaveAttribute('data-fixture-retry', 'true');

  await page.goto(`${fixture}?limited`);
  await expect(page.getByRole('alert')).toContainText('Wait a minute and try again.');
  await expect(page.getByRole('alert')).not.toContainText('public_rooms.rate_limited');

  await page.goto(`${fixture}?loading`);
  await expect(page.getByRole('status')).toHaveText('Loading public rooms…');

  await page.goto(`${fixture}?empty&signed-out`);
  await expect(page.getByRole('status')).toContainText('No rooms found');
  await expect(page.getByRole('button', { name: 'Open a public room' })).toHaveCount(0);
  expect(failures).toEqual([]);
});

test('发现页：再看一些', async ({ page }) => {
  await page.goto(fixture);
  await page.getByRole('button', { name: 'Show more' }).click();
  await expect(card(page, 'Late arrivals')).toBeVisible();
  await expect(page.getByRole('button', { name: 'Show more' })).toHaveCount(0);
});

test('发现页：中文', async ({ browser }) => {
  const context = await browser.newContext({ locale: 'zh-CN' });
  const page = await context.newPage();
  await page.goto(fixture);
  await expect(page.getByRole('heading', { level: 1 })).toHaveText('发现公开房间');
  const rust = card(page, 'Rust help desk');
  await expect(rust).toContainText('房主：Grace');
  await expect(rust).toContainText('18 个人');
  await expect(rust).toContainText('直接进');
  await expect(page.getByRole('searchbox')).toHaveAttribute('placeholder', '搜房间、房主');
  await expect(rust.getByRole('button', { name: '加入 · Rust help desk' })).toBeVisible();
  await expectNoHorizontalOverflow(page);
  await context.close();
});

for (const width of [1440, 390]) {
  test(`房间详情：管理员、在线人数、加入方式和加密的说明，点“加入”就进房间：${String(width)}`, async ({
    page,
  }, testInfo) => {
    const failures = collectPageFailures(page);
    await page.setViewportSize({ width, height: width === 390 ? 844 : 1000 });
    await page.goto(fixture);
    await card(page, 'Rust help desk').getByRole('link', { name: 'Rust help desk' }).click();

    const panel = page.getByRole('dialog', { name: 'Rust help desk' });
    await expect(panel).toContainText('Owner: Grace');
    await expect(panel).toContainText('Admins: Lin, Mo');
    await expect(panel).toContainText('3 online now');
    await expect(panel).toContainText('Opened yesterday');
    await expect(panel).toContainText('Anyone can come straight in.');
    await expect(panel).toContainText('so can the server');
    await expect(panel).toContainText('Long questions are welcome.');
    await expectNoHorizontalOverflow(page);
    const scan = await new AxeBuilder({ page })
      .include('.explore-panel')
      .withTags(['wcag2a', 'wcag2aa', 'wcag21a', 'wcag21aa', 'wcag22aa'])
      .analyze();
    expect(scan.violations.map(({ id }) => id)).toEqual([]);
    await page.screenshot({ path: testInfo.outputPath(`explore-room-${String(width)}.png`) });

    await panel.getByRole('button', { name: 'Join · Rust help desk' }).click();
    await expect(page.locator('body')).toHaveAttribute(
      'data-fixture-opened',
      '/lobby/0198c000-0000-7000-8000-000000000001',
    );
    expect(failures).toEqual([]);
  });
}

test('房间详情：留一句话申请，在等批准时能撤回', async ({ page }) => {
  const failures = collectPageFailures(page);
  await page.goto(fixture);
  await card(page, 'Design critiques')
    .getByRole('link', { name: 'Ask to join · Design critiques' })
    .click();

  const panel = page.getByRole('dialog', { name: 'Design critiques' });
  await expect(panel).toContainText('Ask to join; the owner or an admin lets you in.');
  const note = panel.getByRole('textbox', { name: 'A note for the owner (optional)' });
  await note.fill('x'.repeat(201));
  await expect(panel.getByText('201 / 200')).toBeVisible();
  await expect(panel.getByRole('button', { name: 'Ask to join' })).toBeDisabled();
  await note.fill('I review UI for a living 🙂');
  await panel.getByRole('button', { name: 'Ask to join' }).click();
  await expect(panel).toContainText('You asked to join.');
  await expect(page.locator('body')).toHaveAttribute(
    'data-fixture-request',
    'I review UI for a living 🙂',
  );

  await panel.getByRole('button', { name: 'Withdraw request · Design critiques' }).click();
  await expect(
    panel.getByRole('textbox', { name: 'A note for the owner (optional)' }),
  ).toBeVisible();
  await page.keyboard.press('Escape');
  await expect(
    card(page, 'Design critiques').getByRole('link', { name: 'Ask to join · Design critiques' }),
  ).toBeVisible();
  expect(failures).toEqual([]);
});

test('卡片上：撤回申请、没进成时说为什么', async ({ page }) => {
  const failures = collectPageFailures(page);
  await page.goto(fixture);
  const reading = card(page, '周末读书会');
  await reading.getByRole('button', { name: 'Withdraw request · 周末读书会' }).click();
  await expect(reading.getByRole('link', { name: 'Ask to join · 周末读书会' })).toBeVisible();

  const changing = card(page, 'Changing rooms');
  await changing.getByRole('button', { name: 'Join · Changing rooms' }).click();
  await expect(changing.getByRole('alert')).toHaveText(
    'This room now needs approval. Open it and ask to join.',
  );
  expect(failures).toEqual([]);
});

test('房间详情：不公开了的房间', async ({ page }) => {
  await page.goto(`${fixture}?open=Gone00`);
  const panel = page.getByRole('dialog');
  await expect(panel.getByRole('alert')).toContainText('This room isn’t public anymore');
  await panel.getByRole('button', { name: 'Close room details' }).click();
  await expect(page.getByRole('dialog')).toHaveCount(0);
});

test('房间详情：没登录时是“登录后加入”', async ({ page }) => {
  await page.goto(`${fixture}?signed-out&open=Rm0003`);
  const panel = page.getByRole('dialog', { name: 'Design critiques' });
  await panel.getByRole('button', { name: 'Sign in to join · Design critiques' }).click();
  await expect(page.locator('body')).toHaveAttribute(
    'data-fixture-authentication',
    'sign-in:/explore/Rm0003?join=1',
  );
});
