import { siteCopy, type SiteLanguage } from './copy.js';

export const SITE_ORIGIN = 'https://agentroom.chat';
export const APP_ORIGIN = 'https://app.agentroom.chat';
export const API_ORIGIN = 'https://api.agentroom.chat';
export const REPOSITORY_URL = 'https://github.com/rainyflash/agent-room';

/** 构建时给静态文件起的带摘要的地址（`/_site/` 下，Caddy 给它们长缓存）。 */
export type SiteAssets = {
  readonly css: string;
  readonly script: string;
  readonly font: string;
  readonly room: string;
  readonly mark: string;
  readonly icon: string;
  readonly socialPreview: string;
};

export type PageHead = {
  readonly language: SiteLanguage;
  readonly title: string;
  readonly description: string;
  /** 本页的路径，以 `/` 开头、结尾带 `/`；`canonical` 由它和网站地址拼成。 */
  readonly path: string;
  /** 各语言版本的路径，有几种语言就写几种；`hreflang` 由它生成。 */
  readonly alternates?: Partial<Record<SiteLanguage, string>>;
  readonly jsonLd?: readonly object[];
  readonly kind?: 'website' | 'article';
  readonly indexable?: boolean;
};

export type PageContext = {
  readonly assets: SiteAssets;
  /** 这种语言有没有指南：没有就不在顶栏放“指南”。 */
  readonly hasGuides: Readonly<Record<SiteLanguage, boolean>>;
};

export function escapeHtml(text: string): string {
  return text
    .replaceAll('&', '&amp;')
    .replaceAll('<', '&lt;')
    .replaceAll('>', '&gt;')
    .replaceAll('"', '&quot;')
    .replaceAll("'", '&#39;');
}

export function absolute(path: string): string {
  return `${SITE_ORIGIN}${path}`;
}

export function homePath(language: SiteLanguage): string {
  return language === 'en' ? '/' : '/zh/';
}

export function guidesPath(language: SiteLanguage): string {
  return `${homePath(language)}guides/`;
}

export function appUrl(path: string): string {
  return `${APP_ORIGIN}${path}`;
}

/** JSON-LD 放在 `<script>` 里：`<` 一律转义，正文里写了 `</script>` 也拆不开标签。 */
function jsonLdScript(data: object): string {
  const json = JSON.stringify(data).replaceAll('<', '\\u003c');
  return `<script type="application/ld+json">${json}</script>`;
}

function head(page: PageHead, context: PageContext): string {
  const { assets } = context;
  const alternates = Object.entries(page.alternates ?? {}).map(
    ([language, path]) =>
      `<link rel="alternate" hreflang="${language}" href="${escapeHtml(absolute(path))}">`,
  );
  const englishAlternate = page.alternates?.en;
  if (englishAlternate !== undefined) {
    alternates.push(
      `<link rel="alternate" hreflang="x-default" href="${escapeHtml(absolute(englishAlternate))}">`,
    );
  }
  const canonical = absolute(page.path);
  const lines = [
    '<meta charset="utf-8">',
    '<meta name="viewport" content="width=device-width, initial-scale=1">',
    `<title>${escapeHtml(page.title)}</title>`,
    `<meta name="description" content="${escapeHtml(page.description)}">`,
    page.indexable === false ? '<meta name="robots" content="noindex">' : '',
    page.indexable === false ? '' : `<link rel="canonical" href="${escapeHtml(canonical)}">`,
    ...alternates,
    '<meta name="theme-color" content="#247a77">',
    `<meta property="og:type" content="${page.kind ?? 'website'}">`,
    '<meta property="og:site_name" content="Agent Room">',
    `<meta property="og:title" content="${escapeHtml(page.title)}">`,
    `<meta property="og:description" content="${escapeHtml(page.description)}">`,
    `<meta property="og:url" content="${escapeHtml(canonical)}">`,
    `<meta property="og:locale" content="${page.language === 'en' ? 'en_US' : 'zh_CN'}">`,
    `<meta property="og:image" content="${escapeHtml(absolute(assets.socialPreview))}">`,
    '<meta property="og:image:width" content="1280">',
    '<meta property="og:image:height" content="640">',
    '<meta property="og:image:alt" content="Agent Room: people and AI agents together in one room">',
    '<meta name="twitter:card" content="summary_large_image">',
    `<link rel="icon" href="${assets.mark}" type="image/svg+xml">`,
    `<link rel="apple-touch-icon" href="${assets.icon}">`,
    `<link rel="preload" href="${assets.font}" as="font" type="font/woff2" crossorigin>`,
    `<link rel="stylesheet" href="${assets.css}">`,
    // 语言跳转要在页面画出来之前做，所以放在头里、不加 defer；文件很小。
    `<script src="${assets.script}"></script>`,
    ...(page.jsonLd ?? []).map(jsonLdScript),
  ];
  return lines.filter((line) => line !== '').join('\n');
}

function header(page: PageHead, context: PageContext): string {
  const copy = siteCopy[page.language];
  const other: SiteLanguage = page.language === 'en' ? 'zh-CN' : 'en';
  const otherPath = page.alternates?.[other] ?? homePath(other);
  const guides = context.hasGuides[page.language]
    ? `<a href="${guidesPath(page.language)}">${escapeHtml(copy.nav.guides)}</a>`
    : '';
  return `<header class="site-header">
<a class="site-brand" href="${homePath(page.language)}"><img alt="" src="${context.assets.mark}" width="32" height="32"><span>Agent Room</span></a>
<nav class="site-nav" aria-label="${escapeHtml(copy.nav.label)}">
${guides}
<a href="${REPOSITORY_URL}">${escapeHtml(copy.nav.github)}</a>
<a href="${otherPath}" hreflang="${other}" lang="${other}" data-language="${other}">${escapeHtml(copy.nav.otherLanguage)}</a>
</nav>
<a class="site-button site-button--primary site-header__open" href="${appUrl('/connect')}">${escapeHtml(copy.nav.open)}</a>
</header>`;
}

function footer(page: PageHead): string {
  const copy = siteCopy[page.language];
  const selfHosting = `${REPOSITORY_URL}/blob/main/docs/self-hosting.md`;
  return `<footer class="site-footer">
<p class="site-footer__brand">Agent Room · <a href="${SITE_ORIGIN}/">agentroom.chat</a></p>
<ul>
<li><a href="${REPOSITORY_URL}">${escapeHtml(copy.nav.github)}</a></li>
<li><a href="${selfHosting}">${escapeHtml(copy.footer.selfHosting)}</a></li>
<li><a href="${appUrl('/privacy')}">${escapeHtml(copy.footer.privacy)}</a></li>
<li><a href="/agents.txt">${escapeHtml(copy.footer.agentsTxt)}</a></li>
<li><a href="/llms.txt">llms.txt</a></li>
<li>${escapeHtml(copy.footer.license)}</li>
</ul>
</footer>`;
}

export function layout(page: PageHead, context: PageContext, main: string): string {
  const copy = siteCopy[page.language];
  return `<!doctype html>
<html lang="${page.language}">
<head>
${head(page, context)}
</head>
<body>
<a class="site-skip" href="#main">${escapeHtml(copy.skipToContent)}</a>
${header(page, context)}
<main id="main">
${main}
</main>
${footer(page)}
</body>
</html>
`;
}
