import { readFileSync } from 'node:fs';
import { fileURLToPath } from 'node:url';
import { describe, expect, it } from 'vitest';

import { siteFiles, type SiteFile } from './generate.js';
import type { GuidePage, SiteBuild } from './pages.js';
import { siteSources } from './plugin.js';

const baseBuild: SiteBuild = {
  version: '0.1.0-alpha.80',
  builtOn: '2026-10-20',
  downloads: {
    windows: 'https://download.fixture.invalid/windows.exe',
    macos: 'https://download.fixture.invalid/macos.dmg',
  },
  guides: [],
};

const guide = (language: GuidePage['language'], title: string): GuidePage => ({
  slug: 'claude-code-codex-one-room',
  language,
  title,
  description: `${title} description`,
  updated: '2026-10-21',
  html: '<p>Body <a href="/zh/">中文</a></p>',
});

const withGuides: SiteBuild = {
  ...baseBuild,
  guides: [
    guide('en', 'Let Claude Code and Codex talk in one room'),
    guide('zh-CN', '让 Claude Code 和 Codex 在一个房间里对话'),
  ],
};

async function render(build: SiteBuild): Promise<Map<string, string>> {
  const files = await siteFiles(siteSources, build);
  return new Map(
    files.map((file: SiteFile) => [
      file.path,
      typeof file.contents === 'string'
        ? file.contents
        : `<binary ${String(file.contents.length)}>`,
    ]),
  );
}

function page(files: Map<string, string>, path: string): string {
  const html = files.get(path);
  if (html === undefined) throw new Error(`${path} was not generated`);
  return html;
}

function attribute(html: string, pattern: RegExp): string[] {
  return [...html.matchAll(pattern)].map((match) => match[1] ?? '');
}

function jsonLd(html: string): Record<string, unknown>[] {
  return attribute(html, /<script type="application\/ld\+json">(.*?)<\/script>/gu).map(
    (json) => JSON.parse(json) as Record<string, unknown>,
  );
}

/** 站内链接指向的文件：目录取 index.html；`/agents.txt` 由控制面出，不在网站里。 */
function resolves(files: Map<string, string>, href: string): boolean {
  const path = href.split('#')[0] ?? '';
  if (path === '/agents.txt') return true;
  const relative = path.replace(/^\//u, '');
  return files.has(relative.endsWith('/') || relative === '' ? `${relative}index.html` : relative);
}

describe('agentroom.chat 的静态网站', () => {
  it('中英文首页各有自己的标题、简介、canonical，互相标 hreflang', async () => {
    const files = await render(baseBuild);
    const en = page(files, 'index.html');
    const zh = page(files, 'zh/index.html');

    expect(en).toContain('<html lang="en">');
    expect(zh).toContain('<html lang="zh-CN">');
    expect(en).toContain('<link rel="canonical" href="https://agentroom.chat/">');
    expect(zh).toContain('<link rel="canonical" href="https://agentroom.chat/zh/">');
    for (const html of [en, zh]) {
      expect(html).toContain('<link rel="alternate" hreflang="en" href="https://agentroom.chat/">');
      expect(html).toContain(
        '<link rel="alternate" hreflang="zh-CN" href="https://agentroom.chat/zh/">',
      );
      expect(html).toContain(
        '<link rel="alternate" hreflang="x-default" href="https://agentroom.chat/">',
      );
    }
    expect(attribute(en, /<title>(.*?)<\/title>/gu)).not.toEqual(
      attribute(zh, /<title>(.*?)<\/title>/gu),
    );
    expect(en).toContain('<h1>Bring any AI agent into your room with one message.</h1>');
    expect(zh).toContain('<h1>一句话，把任何 AI Agent 请进你的房间。</h1>');
  });

  it('首页正文不靠脚本：用法、接入方式、谁能读到什么都写在 HTML 里', async () => {
    const files = await render(baseBuild);
    const en = page(files, 'index.html');

    expect(en).toContain('Agents on two computers help each other');
    expect(en).toContain('https://agentroom.chat/agents.txt');
    expect(en).toContain('https://api.agentroom.chat/mcp');
    expect(en).toContain('Whoever runs the server can therefore read your rooms');
    expect(en).not.toMatch(/server (?:also )?can(?:no|')t read/iu);
    expect(en).toContain('href="https://download.fixture.invalid/windows.exe"');
    expect(en).toContain('href="https://app.agentroom.chat/connect"');
  });

  it('首页带 WebSite 和 SoftwareApplication 的结构化数据', async () => {
    const files = await render(baseBuild);
    const data = jsonLd(page(files, 'index.html'));

    expect(data.map((item) => item['@type'])).toEqual(['WebSite', 'SoftwareApplication']);
    expect(data[0]).toMatchObject({ name: 'Agent Room', url: 'https://agentroom.chat/' });
    expect(data[1]).toMatchObject({
      name: 'Agent Room',
      softwareVersion: '0.1.0-alpha.80',
      sameAs: ['https://github.com/rainyflash/agent-room'],
      offers: { price: '0' },
    });
  });

  it('只有外链的样式和脚本，没有内联脚本和内联样式：裸域的内容安全策略不放行这些', async () => {
    const files = await render(withGuides);
    for (const [path, html] of files) {
      if (!path.endsWith('.html')) continue;
      const scripts = attribute(html, /<script([^>]*)>/gu);
      expect(
        scripts.every((attrs) => attrs.includes('src="/_site/') || attrs.includes('ld+json')),
        path,
      ).toBe(true);
      expect(html, path).not.toMatch(/<style|\sstyle="/u);
    }
  });

  it('站内链接都能打开', async () => {
    const files = await render(withGuides);
    for (const [path, html] of files) {
      if (!path.endsWith('.html')) continue;
      for (const href of attribute(html, /\s(?:href|src)="(\/[^"]*)"/gu)) {
        expect(resolves(files, href), `${path} → ${href}`).toBe(true);
      }
    }
  });

  it('静态文件带内容摘要，样式里的字体也换成带摘要的地址', async () => {
    const files = await render(baseBuild);
    const css = [...files.keys()].find((path) => /^_site\/site\.[0-9a-f]{12}\.css$/u.test(path));
    const font = [...files.keys()].find((path) =>
      /^_site\/fredoka\.[0-9a-f]{12}\.woff2$/u.test(path),
    );
    expect(css).toBeDefined();
    expect(font).toBeDefined();
    expect(page(files, css ?? '')).toContain(`url(/${font ?? ''})`);
    expect([...files.keys()].some((path) => /^_site\/room\.[0-9a-f]{12}\.png$/u.test(path))).toBe(
      true,
    );
  });

  it('404 页不进索引，两种语言都说', async () => {
    const files = await render(baseBuild);
    const notFound = page(files, '404.html');

    expect(notFound).toContain('<meta name="robots" content="noindex">');
    expect(notFound).not.toContain('rel="canonical"');
    expect(notFound).toContain('Page not found');
    expect(notFound).toContain('找不到这个页面');
  });

  it('站点地图列出每一页和它的语言版本，robots 指向它', async () => {
    const files = await render(withGuides);
    const sitemap = page(files, 'sitemap.xml');
    const locations = attribute(sitemap, /<loc>(.*?)<\/loc>/gu);

    expect(locations).toEqual([
      'https://agentroom.chat/',
      'https://agentroom.chat/zh/',
      'https://agentroom.chat/guides/',
      'https://agentroom.chat/zh/guides/',
      'https://agentroom.chat/guides/claude-code-codex-one-room/',
      'https://agentroom.chat/zh/guides/claude-code-codex-one-room/',
    ]);
    expect(sitemap).toContain(
      '<xhtml:link rel="alternate" hreflang="zh-CN" href="https://agentroom.chat/zh/guides/claude-code-codex-one-room/"/>',
    );
    expect(sitemap).toContain('<lastmod>2026-10-21</lastmod>');
    expect(page(files, 'robots.txt')).toContain('Sitemap: https://agentroom.chat/sitemap.xml');
  });

  it('没有指南时不出指南目录，顶栏也不放“指南”', async () => {
    const files = await render(baseBuild);

    expect(files.has('guides/index.html')).toBe(false);
    expect(page(files, 'index.html')).not.toContain('href="/guides/"');
    expect(attribute(page(files, 'sitemap.xml'), /<loc>(.*?)<\/loc>/gu)).toEqual([
      'https://agentroom.chat/',
      'https://agentroom.chat/zh/',
    ]);
  });

  it('指南页有自己的 canonical、语言版本和 TechArticle', async () => {
    const files = await render(withGuides);
    const article = page(files, 'guides/claude-code-codex-one-room/index.html');

    expect(article).toContain(
      '<link rel="canonical" href="https://agentroom.chat/guides/claude-code-codex-one-room/">',
    );
    expect(article).toContain(
      '<link rel="alternate" hreflang="zh-CN" href="https://agentroom.chat/zh/guides/claude-code-codex-one-room/">',
    );
    expect(article).toContain('<meta property="og:type" content="article">');
    expect(jsonLd(article)[0]).toMatchObject({
      '@type': 'TechArticle',
      headline: 'Let Claude Code and Codex talk in one room',
      dateModified: '2026-10-21',
    });
    expect(page(files, 'index.html')).toContain('href="/guides/claude-code-codex-one-room/"');
    expect(page(files, 'zh/guides/index.html')).toContain(
      '让 Claude Code 和 Codex 在一个房间里对话',
    );
  });

  it('llms.txt 指向接入说明、远程 MCP 和英文指南', async () => {
    const files = await render(withGuides);
    const llms = page(files, 'llms.txt');

    expect(llms.startsWith('# Agent Room\n\n> ')).toBe(true);
    expect(llms).toContain('[agents.txt](https://agentroom.chat/agents.txt)');
    expect(llms).toContain('(https://api.agentroom.chat/mcp)');
    expect(llms).toContain(
      '[Let Claude Code and Codex talk in one room](https://agentroom.chat/guides/claude-code-codex-one-room/)',
    );
    expect(llms).not.toContain('让 Claude Code');
  });

  it('JSON-LD 里的 < 都转义，正文写了 </script> 也拆不开标签', async () => {
    const files = await render({
      ...withGuides,
      guides: [{ ...guide('en', 'Close </script> tags'), description: 'a < b' }],
    });
    const article = page(files, 'guides/claude-code-codex-one-room/index.html');
    const block = attribute(article, /<script type="application\/ld\+json">(.*?)<\/script>/gu);

    expect(block).toHaveLength(1);
    expect(block[0]).not.toContain('<');
  });
});

describe('网站的颜色和应用一样', () => {
  const read = (relative: string) =>
    readFileSync(fileURLToPath(new URL(relative, import.meta.url)), 'utf8');
  const site = read('../../site/site.css');
  const app = read('../../../../packages/ui-system/src/styles.css');

  /** 从 `marker` 开始到下一个顶格的 `}` 为止，里面所有 `--ar-*`、`--color-*` 的取值。 */
  function tokens(css: string, marker: string): Map<string, string> {
    const start = css.indexOf(marker);
    expect(start, marker).toBeGreaterThanOrEqual(0);
    const block = css.slice(start, css.indexOf('\n}', start));
    return new Map(
      [...block.matchAll(/(--(?:ar|color)-[a-z0-9-]+):\s*([^;]+);/gu)].map((match) => [
        match[1] ?? '',
        (match[2] ?? '').trim(),
      ]),
    );
  }

  const dark = '@media (prefers-color-scheme: dark)';
  it.each([
    [
      '亮色',
      tokens(site, ':root {'),
      new Map([...tokens(app, '@theme {'), ...tokens(app, ':root {')]),
    ],
    ['暗色', tokens(site, dark), tokens(app, dark)],
  ])('%s的令牌取值和 ui-system 一样', (_name, siteTokens, appTokens) => {
    expect(siteTokens.size).toBeGreaterThan(10);
    for (const [name, value] of siteTokens) {
      expect(appTokens.get(name), name).toBe(value);
    }
  });
});
