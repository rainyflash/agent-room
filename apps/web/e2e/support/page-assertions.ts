import { expect, type Page } from '@playwright/test';

// Vite 开发服务器从 node_modules 经 @fs/ 路径提供字体，Firefox 偶发取不到。断言要的是
// 「应用没有报错」，开发服务器取字体失败不是应用的错；生产构建把字体作为普通静态资源发出，
// 根本没有这条路径。范围卡死在这一条消息上，别的控制台错误照样会让用例失败。
const devServerFontNoise = /downloadable font: download failed[\s\S]*@fs\//u;

export function collectPageFailures(page: Page): string[] {
  const failures: string[] = [];
  const record = (text: string) => {
    if (!devServerFontNoise.test(text)) failures.push(text);
  };
  page.on('pageerror', (error) => {
    record(error.message);
  });
  page.on('console', (message) => {
    if (message.type() === 'error') {
      record(message.text());
    }
  });
  return failures;
}

export async function expectNoHorizontalOverflow(page: Page): Promise<void> {
  const dimensions = await page.evaluate(() => ({
    clientWidth: document.documentElement.clientWidth,
    scrollWidth: document.documentElement.scrollWidth,
  }));
  expect(dimensions.scrollWidth).toBeLessThanOrEqual(dimensions.clientWidth);
}
