/**
 * agentroom.chat 上的指南（静态网站，specs/site-discovery/design.md）。和 GitHub 仓库一样是项目自己的地址，
 * 自建的服务器也链到这里。
 */
const SITE_ORIGIN = 'https://agentroom.chat';

export function siteGuidesUrl(language: string | undefined): string {
  return language?.startsWith('zh') === true
    ? `${SITE_ORIGIN}/zh/guides/`
    : `${SITE_ORIGIN}/guides/`;
}
