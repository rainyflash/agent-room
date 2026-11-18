import { readFileSync } from 'node:fs';
import { fileURLToPath } from 'node:url';

import { describe, expect, it, vi } from 'vitest';

import { SHARE_CARD_COLORS } from './share-card-canvas';

// 只看抄的那份颜色，不画：图集那一边在 Node 里用不了。
vi.mock('@/features/lobby/scene/studio-assets', () => ({ loadStudioCharacters: vi.fn() }));

describe('邀请卡片的颜色', () => {
  const app = readFileSync(
    fileURLToPath(new URL('../../../../../../packages/ui-system/src/styles.css', import.meta.url)),
    'utf8',
  );

  /** 从 `marker` 开始到下一个顶格的 `}` 为止，里面所有 `--ar-*`、`--color-*` 的取值。 */
  function tokens(marker: string): Map<string, string> {
    const start = app.indexOf(marker);
    expect(start, marker).toBeGreaterThanOrEqual(0);
    const block = app.slice(start, app.indexOf('\n}', start));
    return new Map(
      [...block.matchAll(/(--(?:ar|color)-[a-z0-9-]+):\s*([^;]+);/gu)].map((match) => [
        match[1] ?? '',
        (match[2] ?? '').trim(),
      ]),
    );
  }

  it('和应用白天的令牌一样', () => {
    const day = new Map([...tokens('@theme {'), ...tokens(':root {')]);
    for (const [token, value] of Object.entries(SHARE_CARD_COLORS)) {
      expect(day.get(token), token).toBe(value);
    }
  });
});
