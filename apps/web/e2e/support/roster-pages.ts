import { expect, type Page } from '@playwright/test';

/** Every member stays reachable while each page keeps the rendered list bounded. */
export async function expectCompleteRosterPages(page: Page, count: number): Promise<void> {
  const roster = page.locator('.list-roster:visible');
  const pagination = roster.getByRole('navigation', { name: 'Agent pages' });
  const next = pagination.getByRole('button', { name: 'Next', exact: true });
  const previous = pagination.getByRole('button', { name: 'Previous', exact: true });
  const pageCount = Math.ceil(count / 100);
  const names: string[] = [];
  const sources = new Set<string | null>();

  await expect(roster.getByRole('button', { name: `Members (${String(count)})` })).toBeVisible();
  await expect(previous).toBeDisabled();
  for (let current = 0; current < pageCount; current += 1) {
    await expect(pagination).toContainText(`${String(current + 1)} / ${String(pageCount)}`);
    const portraits = roster.locator('.roster-agent image[data-character-sprite]');
    await expect(portraits).toHaveCount(Math.min(100, count - current * 100));
    for (const text of await roster.locator('.roster-agent__identity strong').allTextContents()) {
      const name = /^Build Agent \d+/u.exec(text)?.[0];
      if (name === undefined) throw new Error(`Unexpected roster member: ${text}`);
      names.push(name);
    }
    for (const source of await portraits.evaluateAll((images) =>
      images.map((image) => image.getAttribute('href')),
    )) {
      expect(source).not.toBeNull();
      expect(source?.length).toBeLessThan(200);
      sources.add(source);
    }
    if (current + 1 < pageCount) await next.click();
  }
  await expect(next).toBeDisabled();
  await expect(previous).toBeEnabled();
  expect(names).toHaveLength(count);
  expect(new Set(names)).toEqual(
    new Set(
      Array.from(
        { length: count },
        (_, index) => `Build Agent ${String(index + 1).padStart(3, '0')}`,
      ),
    ),
  );
  // The resource stays shared across all pages, including a thousand-member room.
  expect(sources.size).toBe(1);
  await previous.click();
  await expect(pagination).toContainText(`${String(pageCount - 1)} / ${String(pageCount)}`);
}
