import AxeBuilder from '@axe-core/playwright';
import { expect, test, type Page } from '@playwright/test';
import { collectPageFailures, expectNoHorizontalOverflow } from './support/page-assertions';
import { placesButton } from './support/workspace-navigation';

// 左栏里的子房间（specs/redesign/sub-rooms.md“网页”）。`?private&subRooms` 是有子房间的整间房，
// `?private&subRoom` 是“Agent Room”下面的一间子房间。
const parentRoom = '/e2e/fixtures/lobby-scene.html?private&subRooms&layout=conversation';
const subRoom = '/e2e/fixtures/lobby-scene.html?private&subRoom&layout=conversation';

async function expectAccessible(page: Page): Promise<void> {
  const accessibility = await new AxeBuilder({ page })
    .withTags(['wcag2a', 'wcag2aa', 'wcag21aa', 'wcag22aa'])
    .analyze();
  expect(accessibility.violations).toEqual([]);
}

test('整间房的左栏：主房间下面是你在的子房间，未分类的在前，分类能收起，这台设备记着', async ({
  page,
}) => {
  const failures = collectPageFailures(page);
  await page.setViewportSize({ width: 1440, height: 900 });
  await page.goto(parentRoom);
  const places = page.getByRole('navigation', { name: 'Sub-rooms', exact: true });
  await expect(places).toBeVisible();
  await expect(page.getByRole('button', { name: 'Main room', exact: true })).toHaveAttribute(
    'aria-current',
    'page',
  );
  // 只放你在的：“Not mine”你没进，在“浏览子房间”里找。
  await expect(places.getByRole('link')).toHaveText([
    /^#?\s*Loose ends/u,
    /Release checklist/u,
    /Backups/u,
  ]);
  await expect(places.getByText('Not mine')).toHaveCount(0);
  // 能直接进的前面是 #，有门的是锁，读得出来。
  const backups = places.getByRole('link', { name: 'Backups', exact: true });
  await expect(backups).toHaveAccessibleDescription('Has a door');
  await expect(
    places.getByRole('link', { name: 'Loose ends' }).locator('.ar-room-glyph'),
  ).toHaveText('#');
  await expectAccessible(page);

  const work = places.getByRole('button', { name: 'Work', exact: true });
  await work.click();
  await expect(work).toHaveAttribute('aria-expanded', 'false');
  await expect(places.getByRole('link', { name: 'Release checklist' })).toHaveCount(0);
  await page.reload();
  await expect(places.getByRole('button', { name: 'Work', exact: true })).toHaveAttribute(
    'aria-expanded',
    'false',
  );
  await expect(places.getByRole('link', { name: 'Release checklist' })).toHaveCount(0);
  await expect(places.getByRole('link', { name: 'Backups' })).toBeVisible();
  expect(failures).toEqual([]);
});

test('子房间里：顶栏写“父房间 / 子房间”，左栏的主房间回父房间，这一间是“你在这儿”；窄了在下拉里', async ({
  page,
}) => {
  const failures = collectPageFailures(page);
  await page.setViewportSize({ width: 1440, height: 900 });
  await page.goto(subRoom);
  const heading = page.getByRole('heading', { level: 1 });
  await expect(heading).toHaveText(/^Agent Room\s*\/\s*Builders Exchange$/u);
  const parentLink = heading.getByRole('link', { name: 'Agent Room', exact: true });
  await expect(parentLink).toHaveAttribute(
    'href',
    /\/lobby\/01990d9e-8400-7000-8000-000000000400$/u,
  );
  // 左栏的“主房间”也回父房间；这一间自己的对话是子房间里的那一行。
  await expect(page.getByRole('link', { name: 'Main room', exact: true })).toHaveAttribute(
    'href',
    /\/lobby\/01990d9e-8400-7000-8000-000000000400$/u,
  );
  const places = page.getByRole('navigation', { name: 'Sub-rooms', exact: true });
  const here = places.getByRole('button', { name: 'Builders Exchange', exact: true });
  await expect(here).toHaveAttribute('aria-current', 'page');
  // 最上面“浏览子房间”去父房间的那一页。
  await expect(places.getByRole('link')).toHaveText([
    /Browse sub-rooms/u,
    /^#?\s*Loose ends/u,
    /Release checklist/u,
    /Backups/u,
  ]);
  await expect(places.getByRole('link', { name: /^Browse sub-rooms/u })).toHaveAttribute(
    'href',
    /\/lobby\/01990d9e-8400-7000-8000-000000000400\?view=sub-rooms$/u,
  );
  await expectAccessible(page);

  // 窄了：左栏收进房间名旁边的下拉，上面写着现在在这间子房间里。
  await page.setViewportSize({ width: 1000, height: 760 });
  await expect(places).toBeHidden();
  await expectNoHorizontalOverflow(page);
  const menuButton = placesButton(page);
  await expect(menuButton).toHaveAccessibleName(
    'Go to the main room or a direct message (now in Builders Exchange)',
  );
  await menuButton.click();
  const menu = page.getByRole('dialog', { name: 'Main room and direct messages', exact: true });
  await expect(
    menu.getByRole('button', { name: 'Builders Exchange', exact: true }),
  ).toHaveAttribute('aria-current', 'page');
  await expect(menu.getByRole('link', { name: 'Release checklist', exact: true })).toBeVisible();
  await expectAccessible(page);
  expect(failures).toEqual([]);
});

test('浏览子房间：从左栏打开，按分类排，申请、撤回、恢复都在这一页；点房间名回到对话', async ({
  page,
}) => {
  const failures = collectPageFailures(page);
  // 这台设备上见过除了 Security 以外的：左栏那一行带一个点，打开那一页以后点就没了。
  await page.addInitScript(() => {
    window.localStorage.setItem(
      'agent-room.sub-rooms.seen.01990d9e-8400-7000-8000-000000000401',
      JSON.stringify([
        '01990d9e-8400-7000-8000-000000000454',
        '01990d9e-8400-7000-8000-000000000455',
      ]),
    );
  });
  await page.setViewportSize({ width: 1440, height: 900 });
  await page.goto(parentRoom);
  const places = page.getByRole('navigation', { name: 'Sub-rooms', exact: true });
  // 名字里带着有几间：恢复一间以后变成 7。
  const browse = places.getByRole('button', { name: /^Browse sub-rooms/u });
  await expect(browse).toHaveAccessibleName('Browse sub-rooms (6)');
  await expect(browse).toHaveAccessibleDescription('1 new you haven’t seen');
  await browse.click();
  await expect(page).toHaveURL(/view=sub-rooms/u);
  await expect(browse).toHaveAttribute('aria-current', 'page');
  await expect(browse).not.toHaveAccessibleDescription('1 new you haven’t seen');
  const title = page.getByRole('heading', { level: 2, name: 'Sub-rooms', exact: true });
  await expect(title).toBeFocused();
  await expect(page.getByRole('heading', { level: 1 })).toHaveText(
    /^Builders Exchange\s*\/\s*Sub-rooms$/u,
  );
  const browser = page.locator('.sub-room-browser');
  await expect(browser.getByRole('heading', { level: 3 })).toHaveText([
    /^Work/u,
    /^Ops/u,
    /^Uncategorized/u,
  ]);
  await expect(browser.getByRole('button', { name: /^Archived\s*3$/u })).toBeVisible();
  await expectAccessible(page);

  // 在等的撤回，再留句话申请一次。
  await browser.getByRole('button', { name: 'Withdraw your request for Alerts' }).click();
  await browser.getByRole('button', { name: 'Ask to join Alerts' }).click();
  const form = browser.getByRole('form', { name: 'Ask to join Alerts' });
  await form.getByRole('textbox').fill('On call this week');
  await expect(form).toContainText('17/200');
  await form.getByRole('button', { name: 'Send request' }).click();
  await expect(form).toHaveCount(0);
  await expect(
    browser.getByRole('button', { name: 'Withdraw your request for Alerts' }),
  ).toBeVisible();
  // 加入没办成（夹具没接上服务器）：就在那一行下面说，错误码收在后面。
  await browser.getByRole('button', { name: 'Join Not mine' }).click();
  await expect(browser.getByRole('alert')).toContainText('sub_room.unavailable');

  // 已归档：恢复以后回到“全部”里原来的分类下面。
  await browser.getByRole('button', { name: /^Archived/u }).click();
  await expect(browser).toContainText('Grace archived it');
  await expect(browser).toContainText('You archived it');
  await expect(browser).toContainText('Archived after 30 quiet days');
  await expectAccessible(page);
  await browser.getByRole('button', { name: 'Restore Handles ran out' }).click();
  await expect(browser.getByRole('button', { name: /^Archived\s*2$/u })).toBeVisible();
  await browser.getByRole('button', { name: /^All/u }).click();
  await expect(browser.getByText('Handles ran out', { exact: true })).toBeVisible();

  // 点顶栏的房间名回对话，焦点落回左栏那一行。
  await page.getByRole('button', { name: /^Back to Builders Exchange/u }).click();
  await expect(page).not.toHaveURL(/view=sub-rooms/u);
  await expect(browse).toHaveAccessibleName('Browse sub-rooms (7)');
  await expect(browse).toBeFocused();
  await expect(page.getByRole('textbox', { name: 'Message', exact: true })).toBeVisible();
  expect(failures).toEqual([]);
});

test('浏览子房间在手机上：一行拆成两层，不横向滚动', async ({ page }) => {
  const failures = collectPageFailures(page);
  await page.setViewportSize({ width: 390, height: 844 });
  await page.goto(`${parentRoom}&view=sub-rooms`);
  const browser = page.locator('.sub-room-browser');
  await expect(browser.getByRole('heading', { level: 2, name: 'Sub-rooms' })).toBeVisible();
  await expect(browser.getByRole('button', { name: 'Join Not mine' })).toBeInViewport();
  await expectNoHorizontalOverflow(page);
  await expectAccessible(page);
  expect(failures).toEqual([]);
});
