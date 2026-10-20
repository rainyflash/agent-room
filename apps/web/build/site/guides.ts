import { readdir, readFile } from 'node:fs/promises';
import path from 'node:path';
import { Marked } from 'marked';

import { siteLanguages, type SiteLanguage } from './copy.js';
import type { GuidePage } from './pages.js';

/** `<slug>.<语言>.md`，slug 只用小写字母、数字和连字符，就是网址里的那一段。 */
const GUIDE_FILE = /^([a-z0-9]+(?:-[a-z0-9]+)*)\.(en|zh-CN)\.md$/u;
const FRONT_MATTER = /^---\n([\s\S]*?)\n---\n/u;
const DATE = /^\d{4}-\d{2}-\d{2}$/u;

const markdown = new Marked({ gfm: true });

function isLanguage(value: string): value is SiteLanguage {
  return (siteLanguages as readonly string[]).includes(value);
}

/**
 * 一篇指南的一种语言。开头的 front matter 写 `title`、`description`、`updated`（`YYYY-MM-DD`）
 * 和 `order`（指南目录里的先后），正文是 Markdown。
 */
export function parseGuide(
  fileName: string,
  source: string,
): GuidePage & { readonly order: number } {
  const name = GUIDE_FILE.exec(fileName);
  const slug = name?.[1];
  const language = name?.[2];
  if (slug === undefined || language === undefined || !isLanguage(language)) {
    throw new Error(`${fileName}: guide files are named <slug>.en.md or <slug>.zh-CN.md`);
  }
  const text = source.replaceAll('\r\n', '\n');
  const header = FRONT_MATTER.exec(text);
  if (header === null) throw new Error(`${fileName}: front matter is missing`);
  const fields = new Map(
    (header[1] ?? '').split('\n').map((line) => {
      const colon = line.indexOf(':');
      return [line.slice(0, colon).trim(), line.slice(colon + 1).trim()] as const;
    }),
  );
  const field = (key: string): string => {
    const value = fields.get(key);
    if (value === undefined || value === '') throw new Error(`${fileName}: ${key} is missing`);
    return value;
  };
  const updated = field('updated');
  if (!DATE.test(updated)) throw new Error(`${fileName}: updated must be YYYY-MM-DD`);
  const order = Number(field('order'));
  if (!Number.isInteger(order)) throw new Error(`${fileName}: order must be a whole number`);
  return {
    slug,
    language,
    title: field('title'),
    description: field('description'),
    updated,
    order,
    html: markdown.parse(text.slice(header[0].length), { async: false }).trim(),
  };
}

/** 读出 `site/guides/` 下所有指南。每篇都要有中英文两份，少一份就构建失败。 */
export async function loadGuides(directory: string): Promise<GuidePage[]> {
  const files = (await readdir(directory)).filter((file) => file.endsWith('.md'));
  const guides = await Promise.all(
    files.map(async (file) => parseGuide(file, await readFile(path.join(directory, file), 'utf8'))),
  );
  const slugs = new Set(guides.map((guide) => guide.slug));
  for (const slug of slugs) {
    for (const language of siteLanguages) {
      if (!guides.some((guide) => guide.slug === slug && guide.language === language)) {
        throw new Error(`Guide ${slug} has no ${language} version`);
      }
    }
  }
  return guides
    .sort(
      (left, right) =>
        left.order - right.order ||
        left.slug.localeCompare(right.slug) ||
        siteLanguages.indexOf(left.language) - siteLanguages.indexOf(right.language),
    )
    .map(({ slug, language, title, description, updated, html }) => ({
      slug,
      language,
      title,
      description,
      updated,
      html,
    }));
}
