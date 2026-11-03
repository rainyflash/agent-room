import { expect, test } from '@playwright/test';

import { collectPageFailures, expectNoHorizontalOverflow } from './support/page-assertions';
import { openRoomSettings } from './support/workspace-navigation';

/** 私人房间交给了 Agent 管：一份在用的授权、它照规则禁言了一个人。 */
const fixturePath = '/e2e/fixtures/lobby-scene.html?private&agentGrant';

test('交给 Agent 管：台账写是哪个 Agent 照规则做的，再交一次盖掉上一份，一份授权下的一次全撤', async ({
  page,
}) => {
  const failures = collectPageFailures(page);
  await page.setViewportSize({ height: 900, width: 1_440 });
  await page.goto(fixturePath);
  const settings = await openRoomSettings(page, 'Moderation');

  await expect(
    settings.getByText(
      'Fixture Codex Agent, following the rules: Posted the same link five times in a minute.',
    ),
  ).toBeVisible();
  const grants = settings.getByRole('region', { name: 'Agents moderating this room' });
  await expect(grants.getByText(/given by you/u)).toBeVisible();
  await expect(grants.getByText('Mute, Remove from room')).toBeVisible();

  await settings.getByLabel('Rules').fill('Mute anyone who keeps posting ads.');
  await settings.getByRole('checkbox', { exact: true, name: 'Ban' }).check();
  await settings.getByRole('button', { name: 'Hand it over' }).click();
  await expect(grants.getByText('Mute, Ban')).toBeVisible();
  await expect(grants.getByText('Mute, Remove from room')).toHaveCount(0);

  const inEffect = settings.getByRole('region', { name: 'Still in effect from agents' });
  await expect(inEffect.getByText('1 action still in effect')).toBeVisible();
  await inEffect.getByRole('button', { name: 'Undo all' }).click();
  await expect(
    inEffect.getByText('Undo the 1 action Fixture Codex Agent took that is still in effect?'),
  ).toBeVisible();
  await inEffect.getByRole('button', { name: 'Undo all' }).click();
  await expect(inEffect.getByText('Undid 1 action.')).toBeVisible();
  await expect(settings.getByRole('img', { name: 'Undone' })).toBeVisible();

  await grants.getByRole('button', { name: 'Revoke' }).click();
  await expect(grants.getByText('No agent is moderating this room.')).toBeVisible();
  await expectNoHorizontalOverflow(page);

  await page.setViewportSize({ height: 844, width: 390 });
  await settings.getByRole('heading', { name: 'Let an agent moderate' }).scrollIntoViewIfNeeded();
  await expectNoHorizontalOverflow(page);
  expect(failures).toEqual([]);
  await page.screenshot({
    animations: 'disabled',
    path: '../../artifacts/browser/public-rooms/agent-moderation-narrow.png',
  });
});
