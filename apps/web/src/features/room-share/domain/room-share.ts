/**
 * 分享一间公开房间（`specs/public-rooms/design.md`“分享链接”）：一条链接、一段能改的话、发到 X、微博、
 * Reddit 的分享网址和二维码。大家分享的是同一个链接，链接里没有谁分享的。
 */
export type ShareableRoom = {
  readonly name: string;
  readonly description: string;
  /** 服务器拼好的整条分享链接（裸域上的 `/r/<分享码>`）。 */
  readonly shareUrl: string;
};

/** 默认那段话里简介最多取这么多字。 */
export const SHARE_MESSAGE_EXCERPT_CHARS = 60;
/** 预览里的简介和分享页的 `og:description` 一样长。 */
export const SHARE_PREVIEW_CHARS = 150;
/** Reddit 的标题最多 300 个字。 */
const REDDIT_TITLE_CHARS = 300;

/** 简介压成一行，超过 `max` 个字截短加“…”；没写简介是 `null`。按字算：表情、汉字都算一个。 */
export function shareExcerpt(description: string, max: number): string | null {
  const line = description.split(/\s+/u).filter(Boolean).join(' ');
  if (line === '') return null;
  const characters = Array.from(line);
  return characters.length <= max ? line : `${characters.slice(0, max).join('')}…`;
}

/** 去掉话里的链接：发到社交平台时链接另外带上，免得出现两遍。 */
export function withoutLink(text: string, shareUrl: string): string {
  return text.split(shareUrl).join(' ').split(/\s+/u).filter(Boolean).join(' ');
}

export type ShareTargets = {
  readonly x: string;
  readonly weibo: string;
  readonly reddit: string;
};

/**
 * 三家自己的分享网址：只是打开一个新标签页，不加载它们的脚本，它们也不知道是谁点的。话里没写字时
 * 用房间名。
 */
export function shareTargets(room: ShareableRoom, text: string): ShareTargets {
  const words = withoutLink(text, room.shareUrl) || room.name;
  const x = new URL('https://x.com/intent/post');
  x.searchParams.set('text', words);
  x.searchParams.set('url', room.shareUrl);
  const weibo = new URL('https://service.weibo.com/share/share.php');
  weibo.searchParams.set('url', room.shareUrl);
  weibo.searchParams.set('title', words);
  const reddit = new URL('https://www.reddit.com/submit');
  reddit.searchParams.set('url', room.shareUrl);
  reddit.searchParams.set('title', Array.from(words).slice(0, REDDIT_TITLE_CHARS).join(''));
  return { x: x.toString(), weibo: weibo.toString(), reddit: reddit.toString() };
}
