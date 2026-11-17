/**
 * 分享一间公开房间（`specs/public-rooms/design.md`“分享链接”）：一条链接、一段能改的话、发到 X、微博、
 * Reddit 的分享网址和二维码。大家分享的是同一个链接，链接里没有谁分享的。
 */
export type ShareableRoom = {
  readonly name: string;
  readonly description: string;
  /** 服务器拼好的整条分享链接（裸域上的 `/r/<分享码>`）。 */
  readonly shareUrl: string;
  /** 链接里的分享码，拿它去要这间房自己的预览图；不知道时是 `null`，只给通用图。 */
  readonly shareCode: string | null;
};

/** 分享码：6 位大小写字母和数字（控制面 `RoomShareCode`）。不像的不去问服务器。 */
export const SHARE_CODE_PATTERN = /^[A-Za-z0-9]{6}$/u;

/**
 * 这间房自己的预览图（`specs/redesign/project-rooms.md`“预览图”，3P-8b-2）：控制面的
 * `GET /r/<分享码>/preview.png`，和分享页的 `og:image` 是同一张。
 */
export type SharePreviewGateway = {
  /**
   * 此刻的那张 PNG。没有（不公开了、服务器忙着给通用图、连不上、超时）一律是 `null`，界面照旧放通用图；
   * 不抛错。`signal` 停了也是 `null`。
   */
  roomImage(shareCode: string, signal: AbortSignal): Promise<Blob | null>;
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

/**
 * 通用的那张预览图：这间房自己的图要不到时放它，和控制面画不了时 302 去的是同一张。控制面看房间名里有没有
 * 中文字（`share_page.rs` 的 `guess_language`）选语言，这里照同一条规则，预览才和发出去的一样。
 */
export function sharePreviewImage(roomName: string): string {
  return /[\u3400-\u4DBF\u4E00-\u9FFF\uF900-\uFAFF]/u.test(roomName)
    ? '/room-share-preview-zh-CN.png'
    : '/room-share-preview-en.png';
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
export function shareTargets(
  room: Pick<ShareableRoom, 'name' | 'shareUrl'>,
  text: string,
): ShareTargets {
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
