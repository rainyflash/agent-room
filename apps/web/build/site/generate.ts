import { createHash } from 'node:crypto';
import { mkdir, readFile, rm, writeFile } from 'node:fs/promises';
import path from 'node:path';

import { siteLanguages, type SiteLanguage } from './copy.js';
import type { PageContext, SiteAssets } from './html.js';
import {
  guideArticlePage,
  guidePath,
  guidesIndexPage,
  landingPage,
  llmsTxt,
  notFoundPage,
  robots,
  sitemap,
  type SiteBuild,
} from './pages.js';

export type SiteSources = {
  /** `apps/web/site`：样式、脚本、插图。 */
  readonly siteDir: string;
  /** `apps/web/public`：标志、图标、分享预览图，和应用用同一份。 */
  readonly publicDir: string;
  /** Fredoka 的拉丁字符子集（woff2）。 */
  readonly fontFile: string;
};

/** 要写出的一个文件：相对网站根目录的路径和内容。 */
export type SiteFile = { readonly path: string; readonly contents: string | Buffer };

function digest(contents: string | Buffer): string {
  return createHash('sha256').update(contents).digest('hex').slice(0, 12);
}

function hashed(name: string, extension: string, contents: string | Buffer): SiteFile {
  return { path: `_site/${name}.${digest(contents)}.${extension}`, contents };
}

async function assetFiles(
  sources: SiteSources,
): Promise<{ assets: SiteAssets; files: SiteFile[] }> {
  const font = hashed('fredoka', 'woff2', await readFile(sources.fontFile));
  const stylesheet = (await readFile(path.join(sources.siteDir, 'site.css'), 'utf8')).replace(
    'url(fredoka.woff2)',
    `url(/${font.path})`,
  );
  const css = hashed('site', 'css', stylesheet);
  const script = hashed(
    'site',
    'js',
    await readFile(path.join(sources.siteDir, 'site.js'), 'utf8'),
  );
  const room = hashed('room', 'png', await readFile(path.join(sources.siteDir, 'room.png')));
  const fixed: SiteFile[] = [
    {
      path: '_site/mark.svg',
      contents: await readFile(path.join(sources.publicDir, 'agent-room-mark.svg')),
    },
    {
      path: '_site/icon-180.png',
      contents: await readFile(path.join(sources.publicDir, 'icons', '180x180.png')),
    },
    {
      path: '_site/social-preview.png',
      contents: await readFile(path.join(sources.publicDir, 'social-preview.png')),
    },
  ];
  const [mark, icon, socialPreview] = fixed.map((file) => `/${file.path}`);
  if (mark === undefined || icon === undefined || socialPreview === undefined) {
    throw new Error('Static site assets are incomplete.');
  }
  return {
    assets: {
      css: `/${css.path}`,
      script: `/${script.path}`,
      font: `/${font.path}`,
      room: `/${room.path}`,
      mark,
      icon,
      socialPreview,
    },
    files: [font, css, script, room, ...fixed],
  };
}

function pagePath(urlPath: string): string {
  return `${urlPath.replace(/^\//u, '')}index.html`;
}

/** 整个网站要写出的文件，不碰磁盘以外的东西，测试直接看它。 */
export async function siteFiles(sources: SiteSources, build: SiteBuild): Promise<SiteFile[]> {
  const { assets, files } = await assetFiles(sources);
  const hasGuides = Object.fromEntries(
    siteLanguages.map((language) => [
      language,
      build.guides.some((guide) => guide.language === language),
    ]),
  ) as Record<SiteLanguage, boolean>;
  const context: PageContext = { assets, hasGuides };
  const pages: SiteFile[] = siteLanguages.map((language) => ({
    path: pagePath(language === 'en' ? '/' : '/zh/'),
    contents: landingPage(language, context, build),
  }));
  for (const language of siteLanguages.filter((candidate) => hasGuides[candidate])) {
    pages.push({
      path: pagePath(language === 'en' ? '/guides/' : '/zh/guides/'),
      contents: guidesIndexPage(language, context, build),
    });
  }
  for (const guide of build.guides) {
    pages.push({
      path: pagePath(guidePath(guide.language, guide.slug)),
      contents: guideArticlePage(guide, context, build),
    });
  }
  return [
    ...files,
    ...pages,
    { path: '404.html', contents: notFoundPage(context) },
    { path: 'sitemap.xml', contents: sitemap(build) },
    { path: 'robots.txt', contents: robots() },
    { path: 'llms.txt', contents: llmsTxt(build) },
  ];
}

export async function writeSite(outDir: string, files: readonly SiteFile[]): Promise<void> {
  await rm(outDir, { force: true, recursive: true });
  for (const file of files) {
    const target = path.join(outDir, file.path);
    await mkdir(path.dirname(target), { recursive: true });
    await writeFile(target, file.contents);
  }
}
