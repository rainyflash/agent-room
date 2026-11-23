import AxeBuilder from '@axe-core/playwright';
import { expect, test, type Page } from '@playwright/test';
import { collectPageFailures, expectNoHorizontalOverflow } from './support/page-assertions';
import { placesButton, roomAction } from './support/workspace-navigation';

// 左栏里的子房间（specs/redesign/sub-rooms.md“网页”）。`?private&subRooms` 是有子房间的整间房，
// `?private&subRoom` 是“Agent Room”下面的一间子房间。
const parentRoom = '/e2e/fixtures/lobby-scene.html?private&subRooms&layout=conversation';
const subRoom = '/e2e/fixtures/lobby-scene.html?private&subRoom&layout=conversation';

type FixtureSubRoomActions = {
  readonly creations: readonly {
    readonly access: string;
    readonly catalogId: string;
    readonly categoryId?: string;
    readonly description: string;
    readonly name: string;
  }[];
  readonly broughtAgents: readonly { readonly agentId: string; readonly catalogId: string }[];
  readonly revisions: readonly Record<string, unknown>[];
  readonly archived: readonly string[];
  readonly answers: readonly { readonly principalId: string; readonly decision: string }[];
  readonly managers: readonly { readonly kind: string; readonly principalId: string }[];
  readonly policies: readonly Record<string, unknown>[];
  readonly left: readonly string[];
  readonly inspected: readonly string[];
};

async function fixtureSubRoomActions(page: Page): Promise<FixtureSubRoomActions> {
  return await page.evaluate(
    () =>
      (window as unknown as { __agentRoomFixtureSubRoomActions: FixtureSubRoomActions })
        .__agentRoomFixtureSubRoomActions,
  );
}

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

test('新建子房间：左栏和“浏览子房间”都能打开；名字撞了说出来，建好带上你的 Agent，新的那间进左栏', async ({
  page,
}) => {
  const failures = collectPageFailures(page);
  await page.setViewportSize({ width: 1440, height: 900 });
  await page.goto(parentRoom);
  const places = page.getByRole('navigation', { name: 'Sub-rooms', exact: true });
  const create = places.getByRole('button', { name: 'New sub-room', exact: true });
  await create.click();
  const dialog = page.getByRole('dialog', { name: 'New sub-room' });
  await expect(dialog).toHaveAccessibleDescription('Under Builders Exchange');
  const name = dialog.getByRole('textbox', { name: 'Name' });
  await expect(name).toBeFocused();
  await expect(dialog.getByRole('radio', { name: /^Walk in/u })).toBeChecked();
  await expect(dialog.getByRole('checkbox', { name: /Bring my agents/u })).toBeChecked();
  await expectAccessible(page);

  await name.fill('Release checklist');
  await dialog.getByRole('button', { name: 'Create and go in' }).click();
  await expect(dialog).toContainText('There’s already a sub-room with this name.');
  await name.fill('Backup rework');
  await expect(dialog).not.toContainText('There’s already a sub-room with this name.');
  await dialog.getByRole('textbox', { name: /Purpose/u }).fill('Off-site copies and drills.');
  await dialog.getByRole('combobox', { name: 'Category' }).selectOption({ label: 'Ops' });
  await dialog.getByRole('radio', { name: /^Needs approval/u }).check();
  await dialog.getByRole('button', { name: 'Create and go in' }).click();
  // 夹具在 Matrix 上加入不了：建好了、还没进去，照实说；左栏里已经有它。
  await expect(dialog).toContainText('The sub-room was created, but you couldn’t go in yet.');
  await expect(dialog.getByRole('button', { name: 'Try again' })).toBeVisible();
  await expect(places.getByRole('link', { name: 'Backup rework', exact: true })).toBeVisible();
  const actions = await fixtureSubRoomActions(page);
  const created = actions.creations.at(-1);
  expect(created).toMatchObject({
    access: 'request',
    categoryId: '01990d9e-8400-7000-8000-000000000461',
    description: 'Off-site copies and drills.',
    name: 'Backup rework',
  });
  expect(actions.broughtAgents.length).toBeGreaterThan(0);
  expect(actions.broughtAgents.every((entry) => entry.catalogId === created?.catalogId)).toBe(true);
  await dialog.getByRole('button', { name: 'Cancel' }).click();
  await expect(dialog).toHaveCount(0);
  await expect(create).toBeFocused();

  // “浏览子房间”右上角也能新建。
  await places.getByRole('button', { name: /^Browse sub-rooms/u }).click();
  await page.locator('.sub-room-browser').getByRole('button', { name: 'New sub-room' }).click();
  await expect(dialog).toBeVisible();
  expect(failures).toEqual([]);
});

test('还没进的子房间：浏览页点名字打开它的卡片，撤回、再申请都在卡片上；点房间名回对话', async ({
  page,
}) => {
  const failures = collectPageFailures(page);
  await page.setViewportSize({ width: 1440, height: 900 });
  await page.goto(`${parentRoom}&view=sub-rooms`);
  await page
    .locator('.sub-room-browser')
    .getByRole('button', { name: 'Alerts', exact: true })
    .click();
  await expect(page).toHaveURL(/door=01990d9e-8400-7000-8000-000000000455/u);
  await expect(page).not.toHaveURL(/view=sub-rooms/u);
  const door = page.getByRole('region', { name: 'Alerts', exact: true });
  await expect(door.getByRole('heading', { level: 2, name: 'Alerts' })).toBeFocused();
  await expect(page.getByRole('heading', { level: 1 })).toHaveText(
    /^Builders Exchange\s*\/\s*Alerts$/u,
  );
  // 顶栏那一小句（几个人、几个 Agent）说的是整间房，这一页不写，免得读成 Alerts 的。
  await expect(page.locator('.room-desk__meta')).toHaveCount(0);
  await expect(door).toContainText('Ops · Needs approval to join');
  await expect(door).toContainText('Talk about a firing alert here, not in the main room.');
  await expect(door).toContainText('Grace runs this one');
  await expect(door.getByRole('status')).toHaveText('Waiting for Grace to approve.');
  // 里面有谁、说了什么，进去以后才看得到：卡片上没有成员和消息。
  await expect(door).toContainText('Who’s inside and what’s been said shows up after you join.');
  await expectAccessible(page);

  await door.getByRole('button', { name: 'Withdraw request' }).click();
  await expect(door).toContainText('Grace or the room’s admins can let you in.');
  await door.getByRole('button', { name: 'Ask to join', exact: true }).click();
  const form = door.getByRole('form', { name: 'Ask to join Alerts' });
  await form.getByRole('textbox').fill('On call this week');
  await form.getByRole('button', { name: 'Send request' }).click();
  await expect(form).toHaveCount(0);
  await expect(door.getByRole('status')).toHaveText('Waiting for Grace to approve.');

  await page.getByRole('button', { name: /^Back to Builders Exchange/u }).click();
  await expect(page).not.toHaveURL(/door=/u);
  await expect(door).toHaveCount(0);
  await expect(page.getByRole('textbox', { name: 'Message', exact: true })).toBeVisible();
  expect(failures).toEqual([]);
});

test('子房间的门写在地址里：只能请进来的说清楚，直接能进的一个按钮，归档了的能恢复；没有这一间就当没写', async ({
  page,
}) => {
  const failures = collectPageFailures(page);
  await page.setViewportSize({ width: 1440, height: 900 });
  const doorOf = (suffix: string) =>
    page.goto(`${parentRoom}&door=01990d9e-8400-7000-8000-0000000004${suffix}`);

  await doorOf('56');
  const security = page.getByRole('region', { name: 'Security', exact: true });
  await expect(security).toContainText('Ops · Invite only');
  await expect(security).toContainText('Grace, Ada run this one');
  await expect(security).toContainText(
    'Only an admin can bring people in here. You can’t ask to join.',
  );
  await expect(security.getByRole('button')).toHaveCount(0);
  await expectAccessible(page);

  await doorOf('54');
  const notMine = page.getByRole('region', { name: 'Not mine', exact: true });
  await expect(notMine).toContainText('Work · Anyone in the room can walk in');
  await expect(notMine).toContainText('The room’s owner and admins run this one');
  await notMine.getByRole('button', { name: 'Join this one' }).click();
  // 加入没办成（夹具没接上服务器）：卡片上说，错误码收在后面。
  await expect(notMine.getByRole('alert')).toContainText('sub_room.unavailable');

  // 归档了的：写清楚只能看，能恢复的人有“恢复”；恢复以后回到要申请的那扇门。
  await doorOf('72');
  const handles = page.getByRole('region', { name: 'Handles ran out', exact: true });
  await expect(handles).toContainText(
    'This sub-room is archived. Earlier messages can still be read, and nobody can post.',
  );
  // 归档了的谁都进不去：不写“进去以后”那句。
  await expect(handles).not.toContainText('Your agents can come in once you’re in.');
  await expectAccessible(page);
  await handles.getByRole('button', { name: 'Restore' }).click();
  await expect(handles.getByRole('button', { name: 'Ask to join', exact: true })).toBeVisible();
  await expect(handles).not.toContainText('This sub-room is archived.');
  await expect(handles).toContainText('Your agents can come in once you’re in.');

  // 名单上没有的号（看不到、写错了）：照常是对话。
  await doorOf('99');
  await expect(page.getByRole('textbox', { name: 'Message', exact: true })).toBeVisible();
  await expect(page.locator('.sub-room-door')).toHaveCount(0);
  expect(failures).toEqual([]);
});

test('归档了的子房间：对话上面写着归档了，能恢复的人点“恢复”就回来', async ({ page }) => {
  const failures = collectPageFailures(page);
  await page.setViewportSize({ width: 1440, height: 900 });
  await page.goto(`${subRoom}&archived`);
  const notice = page.locator('.sub-room-archived');
  await expect(notice).toHaveRole('status');
  await expect(notice).toContainText(
    'This sub-room is archived. Earlier messages can still be read, and nobody can post.',
  );
  await expectAccessible(page);
  await notice.getByRole('button', { name: 'Restore' }).click();
  await expect(notice).toHaveCount(0);
  expect(failures).toEqual([]);
});

test('子房间设置：管这一间的人改名字、分类和门，名字撞了说出来；归档以后对话上面写着归档了', async ({
  page,
}) => {
  const failures = collectPageFailures(page);
  await page.setViewportSize({ width: 1440, height: 900 });
  await page.goto(subRoom);
  await page.getByRole('button', { name: 'Room settings', exact: true }).click();
  const dialog = page.getByRole('dialog', { name: 'Sub-room settings' });
  await expect(dialog).toHaveAccessibleDescription('Builders Exchange · Agent Room');
  // 整间房的成员、公开、Agent 进门不在这里。
  await expect(dialog.getByRole('radio', { name: 'This sub-room', exact: true })).toBeChecked();
  await expect(dialog.getByRole('radio', { name: 'Members', exact: true })).toHaveCount(0);
  await expect(dialog.getByRole('radio', { name: 'Agent entry', exact: true })).toHaveCount(0);
  const section = dialog.getByRole('region', { name: 'This sub-room' });
  const save = section.getByRole('button', { name: 'Save' });
  await expect(save).toBeDisabled();
  await expect(section.getByRole('combobox', { name: 'Category' })).toHaveValue(
    '01990d9e-8400-7000-8000-000000000460',
  );
  await expect(
    section.getByRole('checkbox', { name: /Don’t show it in the sub-room list/u }),
  ).toBeDisabled();
  await expectAccessible(page);

  const name = section.getByRole('textbox', { name: 'Name' });
  await name.fill('Release checklist');
  await save.click();
  await expect(section).toContainText('There’s already a sub-room with this name.');
  await name.fill('Restore drills');
  await section.getByRole('combobox', { name: 'Category' }).selectOption({ label: 'Ops' });
  await section.getByRole('radio', { name: /^Invite only/u }).check();
  await expect(section).toContainText('With a door, this sub-room is encrypted.');
  await section.getByRole('checkbox', { name: /Don’t show it in the sub-room list/u }).check();
  await save.click();
  await expect(section.getByRole('status')).toHaveText('Saved');
  await expect(save).toBeDisabled();
  const actions = await fixtureSubRoomActions(page);
  expect(actions.revisions.at(-1)).toMatchObject({
    access: 'invite_only',
    categoryId: '01990d9e-8400-7000-8000-000000000461',
    hidden: true,
    name: 'Restore drills',
  });
  // 名单跟着重问：左栏里这一间换了名字、挪到了 Ops 下面。
  const places = page.getByRole('navigation', { name: 'Sub-rooms', exact: true });
  await expect(places.getByRole('button', { name: 'Restore drills', exact: true })).toBeVisible();

  await section.getByRole('button', { name: 'Archive this sub-room' }).click();
  const confirm = section.getByRole('group', { name: 'Archive “Restore drills”?' });
  await expect(confirm.getByRole('button', { name: 'Cancel' })).toBeFocused();
  await confirm.getByRole('button', { name: 'Archive' }).click();
  await expect(dialog).toHaveCount(0);
  await expect(page.locator('.sub-room-archived')).toContainText(
    'This sub-room is archived. Earlier messages can still be read, and nobody can post.',
  );
  expect((await fixtureSubRoomActions(page)).archived).toHaveLength(1);
  expect(failures).toEqual([]);
});

test('子房间里有人在等：左栏那一行和齿轮上带点，设置最上面同意申请、让敲门的 Agent 进来', async ({
  page,
}) => {
  const failures = collectPageFailures(page);
  await page.setViewportSize({ width: 1440, height: 900 });
  await page.goto(`${subRoom}&waiting&knock`);
  const places = page.getByRole('navigation', { name: 'Sub-rooms', exact: true });
  await expect(
    places.getByRole('button', { name: 'Builders Exchange', exact: true }),
  ).toHaveAccessibleDescription(/1 waiting to join/u);
  const gear = page.getByRole('button', { name: 'Room settings', exact: true });
  await expect(gear).toHaveAccessibleDescription('Requests to join: 1');
  await gear.click();

  const dialog = page.getByRole('dialog', { name: 'Sub-room settings' });
  const waiting = dialog.getByRole('region', { name: /^Waiting/u });
  await expect(waiting.getByRole('heading')).toHaveText(/^Waiting\s*2$/u);
  await expect(waiting).toContainText('I run the restore drills.');
  await expectAccessible(page);
  await waiting.getByRole('button', { name: 'Approve Mika' }).click();
  await expect(waiting.getByText('Mika', { exact: true })).toHaveCount(0);
  expect((await fixtureSubRoomActions(page)).answers).toEqual([
    { decision: 'approve', principalId: '0198b601-77a1-7bb8-83eb-a8fe68c97e49' },
  ]);
  await waiting.getByRole('button', { name: /^Let Sol in/u }).click();
  await expect(waiting).toContainText('Nobody is waiting to join.');
  // 名单重问以后左栏和齿轮上的点都没了。
  await expect(gear).not.toHaveAccessibleDescription(/Requests to join/u);
  expect(failures).toEqual([]);
});

test('子房间设置：管这一间的人里写着建的人，从里面的人里设一个管理员，再撤掉；不算最近登录过先请人重新登录', async ({
  page,
}) => {
  const failures = collectPageFailures(page);
  const grace = '0198b601-77a1-7bb8-83eb-a8fe68c97e43';
  await page.setViewportSize({ width: 1440, height: 900 });
  await page.goto(subRoom);
  await page.getByRole('button', { name: 'Room settings', exact: true }).click();
  const dialog = page.getByRole('dialog', { name: 'Sub-room settings' });
  const managers = dialog.getByRole('region', { name: 'People who run this one' });
  await expect(managers.getByRole('listitem')).toHaveText([
    /^A\s*Ada\s*You · Owner · Created it$/u,
  ]);

  await managers.getByRole('button', { name: 'Add a manager' }).click();
  const who = managers.getByRole('combobox', { name: 'Who' });
  await expect(who).toBeFocused();
  await expectAccessible(page);
  await who.selectOption({ label: 'Grace' });
  await managers.getByRole('button', { name: 'Make manager' }).click();
  await expect(who).toHaveCount(0);
  const remove = managers.getByRole('button', { name: 'Stop Grace running this sub-room' });
  await expect(remove).toBeVisible();
  // Grace 已经在管了，没有别人能设：“设管理员”收起来。
  await expect(managers.getByRole('button', { name: 'Add a manager' })).toHaveCount(0);
  await remove.click();
  await expect(managers.getByText('Grace', { exact: true })).toHaveCount(0);
  expect((await fixtureSubRoomActions(page)).managers).toEqual([
    { kind: 'grant', principalId: grace },
    { kind: 'revoke', principalId: grace },
  ]);

  await page.goto(`${subRoom}&olderSession`);
  await page.getByRole('button', { name: 'Room settings', exact: true }).click();
  const older = page
    .getByRole('dialog', { name: 'Sub-room settings' })
    .getByRole('region', { name: 'People who run this one' });
  await older.getByRole('button', { name: 'Add a manager' }).click();
  await expect(older).toContainText('This needs a fresh sign-in.');
  await expect(older.getByRole('button', { name: 'Sign in again' })).toBeVisible();
  await expect(older.getByRole('combobox', { name: 'Who' })).toHaveCount(0);
  expect(failures).toEqual([]);
});

test('房间设置的“子房间”一节：房主改谁能建、自动归档，建分类、改名、挪、删，左栏跟着变', async ({
  page,
}) => {
  const failures = collectPageFailures(page);
  await page.setViewportSize({ width: 1440, height: 900 });
  await page.goto(parentRoom);
  await page.getByRole('button', { name: 'Room settings', exact: true }).click();
  const dialog = page.getByRole('dialog', { name: 'Room settings' });
  await dialog.getByRole('radio', { name: 'Sub-rooms', exact: true }).click();
  const section = dialog.getByRole('region', { name: 'Sub-rooms', exact: true });
  await expect(section.getByRole('switch', { name: /^Agents can create/u })).toBeChecked();
  await expectAccessible(page);

  await section.getByRole('radio', { name: 'Admins only' }).click();
  await expect(section.getByRole('radio', { name: 'Admins only' })).toBeChecked();
  await section.getByRole('switch', { name: /^Archive after 30 days/u }).click();
  await expect(section.getByRole('switch', { name: /^Archive after 30 days/u })).not.toBeChecked();
  expect((await fixtureSubRoomActions(page)).policies).toEqual([
    { creators: 'admins', agentsMayCreate: true, autoArchive: true },
    { creators: 'admins', agentsMayCreate: true, autoArchive: false },
  ]);

  const categories = section.getByRole('region', { name: /^Categories/u });
  const names = categories.locator('li strong');
  await expect(names).toHaveText(['Work', 'Ops', 'Uncategorized']);
  // 新建：Esc 只收起这一行，不关设置。
  await categories.getByRole('button', { name: 'New category' }).click();
  await categories.getByRole('textbox', { name: 'Name of the new category' }).press('Escape');
  await expect(dialog).toBeVisible();
  await categories.getByRole('button', { name: 'New category' }).click();
  await categories.getByRole('textbox', { name: 'Name of the new category' }).fill('work');
  await categories.getByRole('button', { name: 'Create' }).click();
  await expect(categories).toContainText('There’s already a category with this name.');
  await categories.getByRole('textbox', { name: 'Name of the new category' }).fill('Docs');
  await categories.getByRole('button', { name: 'Create' }).click();
  await expect(names).toHaveText(['Work', 'Ops', 'Docs', 'Uncategorized']);

  await categories.getByRole('button', { name: 'Rename Ops' }).click();
  await categories.getByRole('textbox', { name: 'Rename Ops' }).fill('Infra');
  await categories.getByRole('button', { name: 'Rename', exact: true }).click();
  await categories.getByRole('button', { name: 'Move Infra up' }).click();
  await expect(names).toHaveText(['Infra', 'Work', 'Docs', 'Uncategorized']);
  await categories.getByRole('button', { name: 'Delete Work' }).click();
  const confirm = categories.getByRole('group', { name: 'Delete “Work”?' });
  await expect(confirm).toContainText('go back to Uncategorized');
  await confirm.getByRole('button', { name: 'Delete', exact: true }).click();
  await expect(names).toHaveText(['Infra', 'Docs', 'Uncategorized']);

  // 左栏的分类跟着改了名。
  await dialog.getByRole('button', { name: 'Close room settings' }).click();
  const places = page.getByRole('navigation', { name: 'Sub-rooms', exact: true });
  await expect(places.getByRole('button', { name: 'Infra', exact: true })).toBeVisible();
  await expect(places.getByRole('button', { name: 'Work', exact: true })).toHaveCount(0);

  // 手机上：设置开着时窄下来，这一节不横向滚动。
  await page.getByRole('button', { name: 'Room settings', exact: true }).click();
  await page
    .getByRole('dialog', { name: 'Room settings' })
    .getByRole('radio', { name: 'Sub-rooms', exact: true })
    .click();
  await page.setViewportSize({ width: 390, height: 844 });
  await expect(page.getByRole('region', { name: /^Categories/u })).toBeVisible();
  await expectNoHorizontalOverflow(page);
  expect(failures).toEqual([]);
});

test('左栏那一行的“⋯”：标为已读、免打扰、移到分类、别的子房间的设置、归档；房主没有“退出”', async ({
  page,
}) => {
  const failures = collectPageFailures(page);
  await page.setViewportSize({ width: 1440, height: 900 });
  await page.goto(`${parentRoom}&unread`);
  const places = page.getByRole('navigation', { name: 'Sub-rooms', exact: true });
  const release = places.getByRole('link', { name: 'Release checklist', exact: true });
  await expect(release).toHaveAccessibleDescription('Mentions you, 3 unread');

  // 指过去才露出来。
  const more = places.getByRole('button', { name: 'Options for Release checklist' });
  await expect(more).toHaveCSS('opacity', '0');
  await release.hover();
  await expect(more).toHaveCSS('opacity', '1');
  await more.click();
  let menu = page.getByRole('dialog', { name: 'Options for Release checklist' });
  // 你是房主：在每一间里，退不出去。
  await expect(menu.getByRole('button')).toHaveText([
    'Mark as read',
    /^Mute/u,
    'Move to category',
    'Sub-room settings',
    'Archive',
  ]);
  await expectAccessible(page);
  await menu.getByRole('button', { name: 'Mark as read' }).click();
  await expect(menu).toHaveCount(0);
  await expect(more).toBeFocused();
  await expect(release).not.toHaveAccessibleDescription(/unread|Mentions you/u);

  // 免打扰：没读的不数了。
  const backups = places.getByRole('link', { name: 'Backups', exact: true });
  await expect(backups).toHaveAccessibleDescription('Has a door 2 unread');
  await places.getByRole('button', { name: 'Options for Backups' }).click();
  await page
    .getByRole('dialog', { name: 'Options for Backups' })
    .getByRole('button', { name: /^Mute/u })
    .click();
  await expect(backups).toHaveAccessibleDescription('Has a door Muted');
  await places.getByRole('button', { name: 'Options for Backups' }).click();
  await page
    .getByRole('dialog', { name: 'Options for Backups' })
    .getByRole('button', { name: 'Unmute' })
    .click();
  await expect(backups).toHaveAccessibleDescription('Has a door 2 unread');

  // 移到分类：菜单里换成分类的名单，Esc 关掉整个菜单。
  await places.getByRole('button', { name: 'Options for Loose ends' }).click();
  menu = page.getByRole('dialog', { name: 'Options for Loose ends' });
  await menu.getByRole('button', { name: 'Move to category' }).click();
  await expect(menu.getByRole('button', { name: 'Uncategorized' })).toHaveAttribute(
    'aria-pressed',
    'true',
  );
  await menu.getByRole('button', { name: 'Ops', exact: true }).click();
  await expect(menu).toHaveCount(0);
  const ops = places.locator('section').filter({ has: page.getByRole('button', { name: 'Ops' }) });
  await expect(ops.getByRole('link')).toHaveText([/Backups/u, /Loose ends/u]);
  expect((await fixtureSubRoomActions(page)).revisions).toEqual([
    {
      subRoomId: '01990d9e-8400-7000-8000-000000000453',
      categoryId: '01990d9e-8400-7000-8000-000000000461',
    },
  ]);
  // 再移回“未分类”：回到最上面。
  await places.getByRole('button', { name: 'Options for Loose ends' }).click();
  menu = page.getByRole('dialog', { name: 'Options for Loose ends' });
  await menu.getByRole('button', { name: 'Move to category' }).click();
  await menu.getByRole('button', { name: 'Uncategorized' }).click();
  await expect(places.getByRole('link')).toHaveText([
    /Loose ends/u,
    /Release checklist/u,
    /Backups/u,
  ]);

  // 别的子房间的设置：只有“这间子房间”一节。
  await places.getByRole('button', { name: 'Options for Backups' }).click();
  await page
    .getByRole('dialog', { name: 'Options for Backups' })
    .getByRole('button', { name: 'Sub-room settings' })
    .click();
  const settings = page.getByRole('dialog', { name: 'Sub-room settings' });
  await expect(settings).toHaveAccessibleDescription('Backups · Builders Exchange');
  await expect(settings.getByRole('radiogroup', { name: 'Room settings sections' })).toHaveCount(0);
  await expect(settings.getByRole('textbox', { name: 'Name' })).toHaveValue('Backups');
  await settings.getByRole('button', { name: 'Close room settings' }).click();

  // 归档：先问一句，归档了就不在左栏里了。
  await places.getByRole('button', { name: 'Options for Release checklist' }).click();
  menu = page.getByRole('dialog', { name: 'Options for Release checklist' });
  await menu.getByRole('button', { name: 'Archive' }).click();
  const confirm = menu.getByRole('group', { name: 'Archive “Release checklist”?' });
  await expect(confirm.getByRole('button', { name: 'Cancel' })).toBeFocused();
  await confirm.getByRole('button', { name: 'Archive' }).click();
  await expect(release).toHaveCount(0);
  expect((await fixtureSubRoomActions(page)).archived).toEqual([
    '01990d9e-8400-7000-8000-000000000451',
  ]);
  expect(failures).toEqual([]);
});

test('左栏那一行的“⋯”：不是房主的人能退出一间，退了就不在左栏里', async ({ page }) => {
  const failures = collectPageFailures(page);
  await page.setViewportSize({ width: 1440, height: 900 });
  await page.goto(`${parentRoom}&asAdmin`);
  const places = page.getByRole('navigation', { name: 'Sub-rooms', exact: true });
  await places.getByRole('button', { name: 'Options for Backups' }).click();
  const menu = page.getByRole('dialog', { name: 'Options for Backups' });
  await menu.getByRole('button', { name: 'Leave' }).click();
  const confirm = menu.getByRole('group', { name: 'Leave “Backups”?' });
  await expect(confirm).toContainText('To come back, you’ll have to ask again.');
  // 取消回到菜单，焦点回到“退出”。
  await confirm.getByRole('button', { name: 'Cancel' }).click();
  await expect(menu.getByRole('button', { name: 'Leave' })).toBeFocused();
  await menu.getByRole('button', { name: 'Leave' }).click();
  await confirm.getByRole('button', { name: 'Leave' }).click();
  await expect(places.getByRole('link', { name: 'Backups', exact: true })).toHaveCount(0);
  expect((await fixtureSubRoomActions(page)).left).toEqual([
    '01990d9e-8400-7000-8000-000000000452',
  ]);
  // 手机宽度：“⋯”一直在，菜单从下面出来，不横向滚动。
  await page.setViewportSize({ width: 390, height: 844 });
  await expectNoHorizontalOverflow(page);
  expect(failures).toEqual([]);
});

test('浏览子房间：管得了的那几行后面有齿轮，你没进的那一间也能改名字、换分类', async ({ page }) => {
  const failures = collectPageFailures(page);
  await page.setViewportSize({ width: 1440, height: 900 });
  await page.goto(`${parentRoom}&view=sub-rooms`);
  const browser = page.locator('.sub-room-browser');
  // 你是整间房的房主：每一间都管得了，你没进去的也一样。
  await expect(browser.getByRole('button', { name: /^Settings for /u })).toHaveCount(6);
  const gear = browser.getByRole('button', { name: 'Settings for Not mine' });
  await gear.click();
  const settings = page.getByRole('dialog', { name: 'Sub-room settings' });
  await expect(settings).toHaveAccessibleDescription('Not mine · Builders Exchange');
  await expect(settings.getByRole('radiogroup', { name: 'Room settings sections' })).toHaveCount(0);
  const section = settings.getByRole('region', { name: 'This sub-room' });
  await expect(section.getByRole('textbox', { name: 'Name' })).toHaveValue('Not mine');
  await expectAccessible(page);

  await section.getByRole('textbox', { name: 'Name' }).fill('Drafts');
  await section.getByRole('combobox', { name: 'Category' }).selectOption({ label: 'Ops' });
  await section.getByRole('button', { name: 'Save' }).click();
  await expect(section.getByRole('status')).toHaveText('Saved');
  expect((await fixtureSubRoomActions(page)).revisions.at(-1)).toMatchObject({
    subRoomId: '01990d9e-8400-7000-8000-000000000454',
    name: 'Drafts',
    categoryId: '01990d9e-8400-7000-8000-000000000461',
  });
  await settings.getByRole('button', { name: 'Close room settings' }).click();
  await expect(settings).toHaveCount(0);
  // 名单跟着重问：换了名字，挪到 Ops 下面。
  const ops = browser
    .locator('.sub-room-browser__group')
    .filter({ has: page.getByRole('heading', { name: /^Ops/u }) });
  await expect(ops.getByText('Drafts', { exact: true })).toBeVisible();
  await expect(browser.getByRole('button', { name: 'Settings for Drafts' })).toBeVisible();
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

  // 新建的对话框：谁能进的三张卡片叠成一列。
  await browser.getByRole('button', { name: 'New sub-room' }).click();
  const dialog = page.getByRole('dialog', { name: 'New sub-room' });
  await expect(dialog.getByRole('radio', { name: /^Invite only/u })).toBeVisible();
  await expectNoHorizontalOverflow(page);
  await expectAccessible(page);

  // 还没进的一间的卡片：铺满一屏宽，按钮够得着。
  await page.goto(`${parentRoom}&door=01990d9e-8400-7000-8000-000000000455`);
  const door = page.getByRole('region', { name: 'Alerts', exact: true });
  await expect(door.getByRole('button', { name: 'Withdraw request' })).toBeInViewport();
  await expectNoHorizontalOverflow(page);
  await expectAccessible(page);
  expect(failures).toEqual([]);
});

test('公共大厅不在你的房间列表里，但它不会是子房间：打开时不单独去问它（问了是 404）', async ({
  page,
}) => {
  const failures = collectPageFailures(page);
  await page.goto('/e2e/fixtures/lobby-scene.html?layout=conversation');
  await expect(page.getByRole('heading', { level: 1 })).toHaveText('Builders Exchange');
  // 房间列表一回来就会去问；等它一会儿，确认没问。
  await page.waitForTimeout(500);
  expect((await fixtureSubRoomActions(page)).inspected).toEqual([]);
  expect(failures).toEqual([]);
});

test('子房间里请网络 Agent：先请它进父房间，话的最后再叫它拿子房间号进来；不去要子房间的口令', async ({
  page,
  context,
}) => {
  // agent-room-ops#112：以前这里给“生成口令并复制”，要的是子房间的口令，服务器回 409。夹具和服务器一样，
  // 拿子房间号问口令、生成口令都回 409，页面上出错就会被下面的 failures 抓到。
  const failures = collectPageFailures(page);
  await context.grantPermissions(['clipboard-read', 'clipboard-write']);
  await page.setViewportSize({ width: 1440, height: 900 });
  await page.goto(subRoom);
  await (await roomAction(page, 'Bring an agent')).click();
  const dialog = page.getByRole('dialog', { name: 'Bring an agent' });

  // 命令行和 MCP 的话按“父房间名/子房间名”找这一间。
  await dialog.getByRole('radio', { name: /^MCP/u }).click();
  await expect(dialog).toContainText('“Agent Room/Builders Exchange”');

  await dialog.getByRole('radio', { name: /^Network/u }).click();
  await expect(
    dialog.getByText(/“Builders Exchange” is a sub-room of “Agent Room”/u),
  ).toBeVisible();
  await dialog.getByRole('button', { name: 'Create code and copy' }).click();
  await expect(dialog.getByRole('alert')).toHaveCount(0);
  const message = await page.evaluate(() => navigator.clipboard.readText());
  expect(message).toMatch(/enter the Agent Room private room “Agent Room” with the code \S+/u);
  expect(message).toMatch(
    /Once you’re in “Agent Room”, go on into its sub-room “Builders Exchange” with the room number [0-9a-f-]{36}: that’s where I am\./u,
  );
  await expectAccessible(page);
  expect(failures).toEqual([]);
});

test('有门的子房间里请网络 Agent：话里叫它敲子房间的门，管这一间的人在对话框里放行', async ({
  page,
}) => {
  const failures = collectPageFailures(page);
  await page.setViewportSize({ width: 1440, height: 900 });
  await page.goto(`${subRoom}&waiting`);
  await (await roomAction(page, 'Bring an agent')).click();
  const dialog = page.getByRole('dialog', { name: 'Bring an agent' });
  await dialog.getByRole('radio', { name: /^Network/u }).click();
  await expect(
    dialog.getByText(
      'When your agent knocks on “Builders Exchange”, it shows up here for you to let in.',
    ),
  ).toBeVisible();
  await expect(dialog.getByText(/Rather not use a code\? Let it knock/u)).toBeVisible();
  expect(failures).toEqual([]);
});
