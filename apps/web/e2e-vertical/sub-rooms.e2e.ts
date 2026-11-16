import { readFile, writeFile } from 'node:fs/promises';
import { isAbsolute } from 'node:path';

import { expect, test, type BrowserContext, type Page } from '@playwright/test';
import { z } from 'zod';

import { collectUnhandledFailures, connectLiveSession } from '../e2e-live/support/live-session';
import {
  closeRoomSettings,
  expectSaid,
  inRoom,
  openConversation,
  openIsolatedPage,
  openRoomSettings,
  roomPathPattern,
  say,
  waitForStep,
  waitUntilDeviceReady,
} from './support/room-pages';

// 子房间（specs/redesign/sub-rooms.md 第 2k-2a 步）：房主在真实网页上建房间、建分类，在下面开一间直接能进的
// 和一间要申请的；第二个人凭邀请链接进来，他的本机 Agent 按“父房间名/子房间名”进那间直接能进的说一句，
// 两边网页都解开，他的左栏里有了这一间。他申请另一间，房主在子房间设置里批准，他进去说话，房主看得到。
// 网页和纵向验收靠三个文件对步子（tools/vertical.py 的 verify_sub_rooms_in_web）。
const guestUsername = process.env.AGENT_ROOM_E2E_USERNAME;
const guestPassword = process.env.AGENT_ROOM_E2E_PASSWORD;
const ownerUsername = process.env.AGENT_ROOM_VERTICAL_SUB_ROOMS_OWNER_USERNAME;
const ownerPassword = process.env.AGENT_ROOM_VERTICAL_SUB_ROOMS_OWNER_PASSWORD;
const parentName = process.env.AGENT_ROOM_VERTICAL_SUB_ROOMS_PARENT;
const openName = process.env.AGENT_ROOM_VERTICAL_SUB_ROOMS_OPEN;
const requestName = process.env.AGENT_ROOM_VERTICAL_SUB_ROOMS_REQUEST;
const categoryName = process.env.AGENT_ROOM_VERTICAL_SUB_ROOMS_CATEGORY;
const joinedPath = process.env.AGENT_ROOM_VERTICAL_SUB_ROOMS_JOINED;
const spokePath = process.env.AGENT_ROOM_VERTICAL_SUB_ROOMS_SPOKE;
const checkedPath = process.env.AGENT_ROOM_VERTICAL_SUB_ROOMS_CHECKED;

const inviteUrlPattern = /https:\/\/app\.agent-room\.localhost:18443\/join#[0-9a-hjkmnp-tv-z]{26}/u;
const spokeSchema = z.object({ agentName: z.string().min(1), text: z.string().min(1) }).strict();

function catalogOf(page: Page): string {
  const catalogId = roomPathPattern.exec(page.url())?.[1];
  if (catalogId === undefined) throw new Error('这一页不在房间里。');
  return catalogId;
}

function places(page: Page) {
  return page.getByRole('navigation', { name: 'Sub-rooms', exact: true });
}

async function createRoom(owner: Page, name: string): Promise<string> {
  await owner.getByRole('link', { name: 'Rooms', exact: true }).click();
  await owner.getByRole('button', { name: 'New room', exact: true }).click();
  const dialog = owner.getByRole('dialog', { name: 'New room' });
  await dialog.getByLabel('Name', { exact: true }).fill(name);
  await dialog.getByRole('button', { name: 'Create and enter', exact: true }).click();
  await expect(owner).toHaveURL(roomPathPattern, { timeout: 90_000 });
  return catalogOf(owner);
}

// 房间设置的“子房间”一节里建一个分类。
async function createCategory(owner: Page, name: string): Promise<void> {
  const settings = await openRoomSettings(owner, 'Sub-rooms');
  const categories = settings.getByRole('region', { name: /^Categories/u });
  await categories.getByRole('button', { name: 'New category' }).click();
  await categories.getByRole('textbox', { name: 'Name of the new category' }).fill(name);
  await categories.getByRole('button', { name: 'Create' }).click();
  await expect(categories.locator('li strong')).toContainText([name]);
  await closeRoomSettings(owner, settings);
}

// 左栏的“新建子房间”：放进分类、选门，不带 Agent（这一轮只看人的；Agent 由纵向验收按名字带进来）。
async function createSubRoom(
  owner: Page,
  parentCatalogId: string,
  name: string,
  door: RegExp,
  category: string | null,
): Promise<string> {
  await owner.goto(`/lobby/${parentCatalogId}`);
  await expect(owner).toHaveURL(inRoom(parentCatalogId), { timeout: 90_000 });
  await places(owner).getByRole('button', { name: 'New sub-room', exact: true }).click();
  const dialog = owner.getByRole('dialog', { name: 'New sub-room' });
  await dialog.getByRole('textbox', { name: 'Name' }).fill(name);
  if (category !== null) {
    await dialog.getByRole('combobox', { name: 'Category' }).selectOption({ label: category });
  }
  await dialog.getByRole('radio', { name: door }).check();
  const bring = dialog.getByRole('checkbox', { name: /Bring my agents/u });
  if ((await bring.count()) > 0 && (await bring.isChecked())) await bring.uncheck();
  await dialog.getByRole('button', { name: 'Create and go in' }).click();
  await expect(owner).not.toHaveURL(inRoom(parentCatalogId), { timeout: 90_000 });
  await expect(owner).toHaveURL(roomPathPattern, { timeout: 90_000 });
  await expect(owner.getByRole('heading', { level: 1 })).toContainText(name, {
    timeout: 60_000,
  });
  return catalogOf(owner);
}

async function inviteLink(owner: Page, parentCatalogId: string): Promise<string> {
  await owner.goto(`/lobby/${parentCatalogId}`);
  await expect(owner).toHaveURL(inRoom(parentCatalogId), { timeout: 90_000 });
  await owner.getByRole('button', { name: 'Invite people', exact: true }).click();
  const dialog = owner.getByRole('dialog', { name: 'Invite people' });
  await expect(dialog).toContainText('There is no link yet.', { timeout: 30_000 });
  await dialog.getByRole('button', { name: 'Make link and copy' }).click();
  await expect(dialog).toContainText(inviteUrlPattern, { timeout: 30_000 });
  const link = inviteUrlPattern.exec(await dialog.innerText())?.[0];
  if (link === undefined) throw new Error('“邀请人”里没有看到链接。');
  await owner.keyboard.press('Escape');
  await expect(dialog).toBeHidden();
  return link;
}

// 已经登录的人打开邀请链接，点“加入”进上面那间房。
async function joinByLink(guest: Page, link: string, parentCatalogId: string): Promise<void> {
  await guest.goto(link);
  await expect(guest.getByRole('heading', { name: /invited you to/u })).toBeVisible({
    timeout: 90_000,
  });
  await guest.getByRole('button', { name: 'Join', exact: true }).click();
  await expect(guest).toHaveURL(inRoom(parentCatalogId), { timeout: 90_000 });
}

// 在上面那间房里等左栏出现这一间（名单隔一会儿才重问，等不到就重新打开），点进去。
async function enterFromPlaces(page: Page, parentCatalogId: string, name: string): Promise<void> {
  const link = places(page).getByRole('link', { name, exact: true });
  await expect
    .poll(
      async () => {
        if (await link.isVisible()) return true;
        await page.goto(`/lobby/${parentCatalogId}`);
        return false;
      },
      { intervals: [5_000], timeout: 180_000 },
    )
    .toBe(true);
  await link.click();
  await expect(page.getByRole('heading', { level: 1 })).toContainText(name, { timeout: 60_000 });
  await openConversation(page);
}

// “浏览子房间”里留句话申请进那一间。
async function askToJoin(guest: Page, parentCatalogId: string, name: string): Promise<void> {
  await guest.goto(`/lobby/${parentCatalogId}`);
  await places(guest)
    .getByRole('button', { name: /^Browse sub-rooms/u })
    .click();
  const browser = guest.locator('.sub-room-browser');
  await browser.getByRole('button', { name: `Ask to join ${name}` }).click();
  const form = browser.getByRole('form', { name: `Ask to join ${name}` });
  await form.getByRole('textbox').fill('I review the plans.');
  await form.getByRole('button', { name: 'Send request' }).click();
  await expect(
    browser.getByRole('button', { name: `Withdraw your request for ${name}` }),
  ).toBeVisible({ timeout: 60_000 });
}

// 管这一间的人在子房间设置最上面批准。
async function approve(owner: Page, catalogId: string, person: string): Promise<void> {
  await owner.goto(`/lobby/${catalogId}`);
  await expect(owner).toHaveURL(inRoom(catalogId), { timeout: 90_000 });
  const gear = owner.getByRole('button', { name: 'Room settings', exact: true });
  await expect(gear).toHaveAccessibleDescription(/Requests to join/u, { timeout: 120_000 });
  await gear.click();
  const dialog = owner.getByRole('dialog', { name: 'Sub-room settings' });
  const waiting = dialog.getByRole('region', { name: /^Waiting/u });
  await expect(waiting).toContainText('I review the plans.');
  await waiting.getByRole('button', { name: `Approve ${person}` }).click();
  await expect(waiting).toContainText('Nobody is waiting to join.', { timeout: 60_000 });
  await owner.keyboard.press('Escape');
  await expect(dialog).toBeHidden();
}

test.use({ actionTimeout: 90_000 });

test('子房间：建分类和两间子房间，第二个人的 Agent 按名字进来，申请的那一间房主批准以后进来说话', async ({
  browser,
}) => {
  test.skip(
    guestUsername === undefined ||
      guestPassword === undefined ||
      ownerUsername === undefined ||
      ownerPassword === undefined ||
      parentName === undefined ||
      openName === undefined ||
      requestName === undefined ||
      categoryName === undefined ||
      joinedPath === undefined ||
      spokePath === undefined ||
      checkedPath === undefined,
    '缺少子房间纵向验收变量。',
  );
  test.setTimeout(1_200_000);
  for (const path of [joinedPath, spokePath, checkedPath]) {
    expect(isAbsolute(path ?? '')).toBe(true);
  }

  const contexts: BrowserContext[] = [];
  try {
    const owner = await openIsolatedPage(browser, contexts);
    const guest = await openIsolatedPage(browser, contexts);
    const failures = {
      guest: collectUnhandledFailures(guest),
      owner: collectUnhandledFailures(owner),
    };

    await connectLiveSession(owner, {
      expectedDisplayName: 'Local Collaborator',
      password: ownerPassword ?? '',
      username: ownerUsername ?? '',
    });
    await waitUntilDeviceReady(owner);
    const parentCatalogId = await createRoom(owner, parentName ?? '');
    await createCategory(owner, categoryName ?? '');
    const openCatalogId = await createSubRoom(
      owner,
      parentCatalogId,
      openName ?? '',
      /^Walk in/u,
      categoryName ?? '',
    );
    const requestCatalogId = await createSubRoom(
      owner,
      parentCatalogId,
      requestName ?? '',
      /^Needs approval/u,
      null,
    );
    const link = await inviteLink(owner, parentCatalogId);

    await connectLiveSession(guest, {
      expectedDisplayName: 'Local Developer',
      password: guestPassword ?? '',
      username: guestUsername ?? '',
    });
    await waitUntilDeviceReady(guest);
    await joinByLink(guest, link, parentCatalogId);
    await writeFile(
      joinedPath ?? '',
      `${JSON.stringify({ openCatalogId, parentCatalogId })}\n`,
      'utf8',
    );

    // 纵向验收让他的本机 Agent 按“父房间名/子房间名”进直接能进的那间说一句：他自己也就跟着进去了。
    await waitForStep(spokePath ?? '');
    const spoke = spokeSchema.parse(JSON.parse(await readFile(spokePath ?? '', 'utf8')));
    await enterFromPlaces(guest, parentCatalogId, openName ?? '');
    await expectSaid(guest, spoke.text);
    await owner.goto(`/lobby/${openCatalogId}`);
    await openConversation(owner);
    await expectSaid(owner, spoke.text);

    await askToJoin(guest, parentCatalogId, requestName ?? '');
    await approve(owner, requestCatalogId, 'Local Developer');
    await enterFromPlaces(guest, parentCatalogId, requestName ?? '');
    const approved = `Thanks for letting me into ${requestName ?? ''}.`;
    await say(guest, approved);
    await owner.goto(`/lobby/${requestCatalogId}`);
    await openConversation(owner);
    await expectSaid(owner, approved);

    expect(failures).toEqual({ guest: [], owner: [] });
    await writeFile(checkedPath ?? '', `${JSON.stringify({ requestCatalogId })}\n`, 'utf8');
  } finally {
    await Promise.all(
      contexts.map(async (context) => {
        await context.close();
      }),
    );
  }
});
