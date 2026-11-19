import type { Page } from '@playwright/test';

/**
 * 换配色以后等颜色过渡走完：走到一半时按钮上的字和底都还在半路上，量出来的对比度不作数。
 * 等到一个过渡都不剩为止：头一批走完时可能又起了新的（颜色是继承来的、自己又带过渡的元素，
 * 上一层每走一帧它就重新起一次）。换了配色接着查对比度的用例都用它，别直接 `emulateMedia`。
 */
export async function useColorScheme(page: Page, colorScheme: 'dark' | 'light'): Promise<void> {
  await page.emulateMedia({ colorScheme });
  await page.evaluate(async () => {
    const running = () =>
      document.getAnimations().filter((animation) => animation instanceof CSSTransition);
    for (let transitions = running(), round = 0; transitions.length > 0; transitions = running()) {
      if ((round += 1) > 100) throw new Error('换配色以后颜色过渡一直没走完。');
      await Promise.all(transitions.map((transition) => transition.finished.catch(() => null)));
    }
  });
}
