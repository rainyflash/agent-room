import { readdirSync, readFileSync } from 'node:fs';
import { dirname, join, resolve } from 'node:path';
import { fileURLToPath } from 'node:url';

import { describe, expect, it } from 'vitest';

/// 样式表是运行时契约的一部分：下面两类错误都不会报错，只会静默退化，肉眼很难发现。
// 不写 new URL('字面量', import.meta.url)：Vite 会把它改写成开发服务器上的资源地址。
const sourceRoot = resolve(dirname(fileURLToPath(import.meta.url)), '..');
const uiSystemStyles = resolve(sourceRoot, '../../../packages/ui-system/src/styles.css');

type Stylesheet = { readonly path: string; readonly css: string };

function stylesheets(): readonly Stylesheet[] {
  const files = readdirSync(sourceRoot, { recursive: true, encoding: 'utf8' })
    .filter((path) => path.endsWith('.css'))
    .map((path) => join(sourceRoot, path));
  return [...files, uiSystemStyles].map((path) => ({ path, css: readFileSync(path, 'utf8') }));
}

type Declaration = {
  readonly media: string | null;
  readonly selector: string;
  readonly property: string;
  readonly value: string;
};

/// 只认一层 @media 嵌套，足够覆盖本项目的样式写法；分组选择器拆成单个选择器比较。
function declarations(css: string): readonly Declaration[] {
  const source = css.replace(/\/\*[\s\S]*?\*\//gu, '');
  const result: Declaration[] = [];
  const stack: { readonly kind: 'at' | 'rule'; readonly head: string }[] = [];
  let buffer = '';
  const flush = (text: string) => {
    const rule = stack.at(-1);
    const colon = text.indexOf(':');
    if (rule?.kind !== 'rule' || colon < 0) return;
    const media = stack.findLast((entry) => entry.kind === 'at')?.head ?? null;
    for (const selector of rule.head.split(',')) {
      result.push({
        media,
        selector: selector.split(/\s+/u).filter(Boolean).join(' '),
        property: text.slice(0, colon).trim(),
        value: text.slice(colon + 1).trim(),
      });
    }
  };
  for (const character of source) {
    if (character === '{') {
      const head = buffer.trim();
      stack.push({ kind: head.startsWith('@') ? 'at' : 'rule', head });
      buffer = '';
    } else if (character === '}') {
      if (buffer.trim() !== '') flush(buffer);
      buffer = '';
      stack.pop();
    } else if (character === ';') {
      flush(buffer);
      buffer = '';
    } else {
      buffer += character;
    }
  }
  return result;
}

describe('样式表契约', () => {
  it('每个引用到的变量都有定义', () => {
    const css = stylesheets()
      .map((sheet) => sheet.css)
      .join('\n');
    const defined = new Set([...css.matchAll(/^\s*(--[a-z0-9-]+)\s*:/gmu)].map(([, name]) => name));
    const referenced = new Set([...css.matchAll(/var\((--[a-z0-9-]+)/gu)].map(([, name]) => name));
    const missing = [...referenced].filter((name) => !defined.has(name)).toSorted();
    expect(missing).toEqual([]);
  });

  it('响应式规则不会被同一文件里后写的无条件规则覆盖', () => {
    // 同一选择器、同一属性：媒体查询里的值写在前面、无条件的值写在后面时，媒体查询永远不生效。
    // 这样的错误让「我的 Agent」和安全页在手机上一直沿用桌面三栏/两栏布局。
    const ignoredMedia = /prefers-reduced-motion|forced-colors|hover|print/u;
    const overridden: string[] = [];
    for (const sheet of stylesheets()) {
      const all = declarations(sheet.css);
      all.forEach((declaration, index) => {
        if (declaration.media?.startsWith('@media') !== true) return;
        if (ignoredMedia.test(declaration.media)) return;
        const later = all
          .slice(index + 1)
          .findLast(
            (candidate) =>
              candidate.media === null &&
              candidate.selector === declaration.selector &&
              candidate.property === declaration.property,
          );
        if (later !== undefined && later.value !== declaration.value) {
          const file = sheet.path.slice(sourceRoot.length - 'src'.length);
          overridden.push(
            `${file}: ${declaration.selector} { ${declaration.property} } in ${declaration.media}`,
          );
        }
      });
    }
    expect(overridden).toEqual([]);
  });
});
