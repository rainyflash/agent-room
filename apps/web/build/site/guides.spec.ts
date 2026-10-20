import { describe, expect, it } from 'vitest';

import { siteFiles } from './generate.js';
import { loadGuides, parseGuide } from './guides.js';
import { siteSources } from './plugin.js';

const source = (fields: string, body = 'Hello **world**.\n') => `---\n${fields}\n---\n\n${body}`;
const complete = 'title: A title\ndescription: A description\norder: 1\nupdated: 2026-10-20';

describe('指南文件', () => {
  it('读出 front matter，正文转成 HTML', () => {
    const guide = parseGuide('one-room.en.md', source(complete).replaceAll('\n', '\r\n'));

    expect(guide).toEqual({
      slug: 'one-room',
      language: 'en',
      title: 'A title',
      description: 'A description',
      order: 1,
      updated: '2026-10-20',
      html: '<p>Hello <strong>world</strong>.</p>',
    });
  });

  it('文件名、front matter 不对就构建失败', () => {
    expect(() => parseGuide('One Room.en.md', source(complete))).toThrow('guide files are named');
    expect(() => parseGuide('one-room.fr.md', source(complete))).toThrow('guide files are named');
    expect(() => parseGuide('one-room.en.md', 'no front matter')).toThrow('front matter');
    expect(() =>
      parseGuide('one-room.en.md', source(complete.replace('title: A title\n', ''))),
    ).toThrow('title is missing');
    expect(() =>
      parseGuide('one-room.en.md', source(complete.replace('2026-10-20', 'October 20'))),
    ).toThrow('updated must be YYYY-MM-DD');
  });
});

describe('仓库里的指南', () => {
  const directory = `${siteSources.siteDir}guides`;

  it('每篇都有中英文两份，按 order 排好', async () => {
    const guides = await loadGuides(directory);
    const slugs = [...new Set(guides.map((guide) => guide.slug))];

    expect(slugs).toEqual([
      'claude-code-codex-one-room',
      'agents-working-together',
      'chatgpt-with-coding-agents',
      'about-the-name',
    ]);
    for (const slug of slugs) {
      expect(guides.filter((guide) => guide.slug === slug).map((guide) => guide.language)).toEqual([
        'en',
        'zh-CN',
      ]);
    }
  });

  it('都生成得出来，站内链接指向生成了的页面', async () => {
    const guides = await loadGuides(directory);
    const files = await siteFiles(siteSources, {
      version: '0.1.0-alpha.80',
      builtOn: '2026-10-20',
      downloads: {
        windows: 'https://example.invalid/w.exe',
        macos: 'https://example.invalid/m.dmg',
      },
      guides,
    });
    const paths = new Set(files.map((file) => file.path));
    for (const guide of guides) {
      for (const [, href] of guide.html.matchAll(/href="(\/[^"#]*)"/gu)) {
        const target = (href ?? '').replace(/^\//u, '');
        expect(paths.has(target.endsWith('/') ? `${target}index.html` : target), href).toBe(true);
      }
    }
  });

  it('说加密照 README 的口径：不说服务器读不到，说运营方读得到', async () => {
    const guides = await loadGuides(directory);
    for (const guide of guides) {
      expect(guide.html, guide.slug).not.toMatch(
        /server (?:also )?can(?:no|')t read|服务器(?:也)?读不到/u,
      );
    }
    const english = guides.filter((guide) => guide.language === 'en');
    expect(english.filter((guide) => guide.html.includes('whoever runs')).length).toBeGreaterThan(
      0,
    );
  });
});
