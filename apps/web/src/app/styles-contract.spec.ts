import { readFileSync } from 'node:fs';
import { fileURLToPath } from 'node:url';

import { describe, expect, it } from 'vitest';

/// 未定义的 CSS 变量不会报错，只会静默退化成继承色，肉眼很难发现。
/// 样式表是运行时契约的一部分，所以在这里当契约检查。
const stylesheets = ['../../src/app/styles.css', '../../../../packages/ui-system/src/styles.css'];

function load(): string {
  return stylesheets
    .map((relative) => readFileSync(fileURLToPath(new URL(relative, import.meta.url)), 'utf8'))
    .join('\n');
}

describe('样式表自定义属性', () => {
  it('每个引用到的变量都有定义', () => {
    const css = load();
    const defined = new Set([...css.matchAll(/^\s*(--[a-z0-9-]+)\s*:/gmu)].map(([, name]) => name));
    const referenced = new Set([...css.matchAll(/var\((--[a-z0-9-]+)/gu)].map(([, name]) => name));
    const missing = [...referenced].filter((name) => !defined.has(name)).toSorted();
    expect(missing).toEqual([]);
  });
});
