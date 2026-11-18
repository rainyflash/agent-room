import { expect, type Locator } from '@playwright/test';
import { encode } from 'uqr';

import { shareCardQrLayout } from '../../src/features/room-share/domain/share-card';

/** 卡片那张图上的二维码，一格一格读出来：格子正中是黑的算黑。 */
async function cardQrModules(card: Locator, modules: number): Promise<boolean[][]> {
  return await card.evaluate(
    (image: HTMLImageElement, layout) => {
      const canvas = document.createElement('canvas');
      canvas.width = image.naturalWidth;
      canvas.height = image.naturalHeight;
      const context = canvas.getContext('2d');
      if (context === null) throw new Error('2D canvas unavailable');
      context.drawImage(image, 0, 0);
      const rows: boolean[][] = [];
      for (let row = 0; row < layout.modules; row += 1) {
        const cells: boolean[] = [];
        for (let column = 0; column < layout.modules; column += 1) {
          const [red = 255, green = 255, blue = 255] = context.getImageData(
            layout.x + column * layout.cell + Math.floor(layout.cell / 2),
            layout.y + row * layout.cell + Math.floor(layout.cell / 2),
            1,
            1,
          ).data;
          cells.push(red + green + blue < 384);
        }
        rows.push(cells);
      }
      return rows;
    },
    { ...shareCardQrLayout(modules), modules },
  );
}

/**
 * 发给人的那张卡片（`specs/share-card/design.md`）画好了：1080×1350，上面的二维码就是这条链接。
 * `card` 是对话框里卡片的那张 `<img>`。改卡片的版式别绕过这里：它照 `shareCardQrLayout` 找码在哪儿。
 */
export async function expectShareCardFor(card: Locator, url: string): Promise<void> {
  const qr = encode(url, { border: 0, ecc: 'M' });
  await expect(async () => {
    expect(
      await card.evaluate((image: HTMLImageElement) => [image.naturalWidth, image.naturalHeight]),
    ).toEqual([1080, 1350]);
    expect(await cardQrModules(card, qr.size)).toEqual(qr.data);
  }).toPass();
}
