import type { RoomTone } from '@/shared/ui/room-tone';

/**
 * 发给人的那张卡片（`specs/share-card/design.md`）：竖的一张图，房间名、一句谁请你来、二维码。
 * 在本机画，不经过服务器。
 */
export const SHARE_CARD_SIZE = Object.freeze({ width: 1080, height: 1350 });

export type ShareCardContent = {
  /** 房间名上面那一行：谁邀请你，或者“公开房间”。 */
  readonly lead: string;
  readonly roomName: string;
  /** 房间小图标的颜色，和应用里是同一种。 */
  readonly tone: RoomTone;
  /** 二维码里的地址。 */
  readonly url: string;
  /** 二维码旁边大的那一句：“扫码进房间”。 */
  readonly action: string;
  /** 那一句下面小一号的几行：登录后就能进来、到什么时候有效。一项一行，放不下才折。 */
  readonly notes: readonly string[];
  /** 右下角网址上面的一句：Agent Room 是什么。 */
  readonly tagline: string;
};

/** 二维码垫的那块白底在卡片上的位置，和它四边留的白。 */
export const SHARE_CARD_QR = Object.freeze({ x: 135, y: 555, size: 440, padding: 28 });

/**
 * 二维码一格多大、从哪儿画起。一格占整数个像素、起点也落在整像素上，边才是齐的；整块摆在白底正中。
 * `modules` 是二维码一边几格。画的那一边和浏览器验收（逐格核对图上的码）都照它算。
 */
export function shareCardQrLayout(modules: number): {
  readonly cell: number;
  readonly x: number;
  readonly y: number;
} {
  const room = SHARE_CARD_QR.size - SHARE_CARD_QR.padding * 2;
  const cell = Math.max(1, Math.floor(room / modules));
  const inset = Math.floor((SHARE_CARD_QR.size - cell * modules) / 2);
  return { cell, x: SHARE_CARD_QR.x + inset, y: SHARE_CARD_QR.y + inset };
}

/** 右下角写的网址：只写主机名，带不带 `www`、路径和 `#` 后面的都不要。 */
export function shareCardSite(url: string): string {
  try {
    return new URL(url).host;
  } catch {
    return '';
  }
}

/** 文件名里系统不让用的字（Windows 的那几个），控制字符另算。 */
const UNSAFE_IN_FILE_NAMES = new Set('\\/:*?"<>|');

/** 存下来的文件名：房间名里系统不让用的字换掉，太长截短。 */
export function shareCardFileName(roomName: string): string {
  const cleaned = Array.from(roomName, (character) =>
    UNSAFE_IN_FILE_NAMES.has(character) || (character.codePointAt(0) ?? 0) < 0x20 ? ' ' : character,
  ).join('');
  const words = cleaned.split(/\s+/u).filter(Boolean).join(' ');
  const safe = Array.from(words).slice(0, 40).join('').trim();
  return safe === '' ? 'agent-room.png' : `agent-room-${safe}.png`;
}

/**
 * 按宽度折行，最多 `maxLines` 行；放不下的，最后一行截短加“…”。英文尽量在空格处断，中文没有空格、
 * 一个字一个字地断。`measure` 量一段字画出来多宽，由画的那一边给。
 */
export function wrapCardText(
  text: string,
  maxWidth: number,
  maxLines: number,
  measure: (text: string) => number,
): readonly string[] {
  const characters = Array.from(text.split(/\s+/u).filter(Boolean).join(' '));
  const lines: string[] = [];
  let line = '';
  let index = 0;
  while (index < characters.length) {
    const character = characters[index] ?? '';
    if (line === '' && character === ' ') {
      index += 1;
      continue;
    }
    if (measure(line + character) <= maxWidth || line === '') {
      line += character;
      index += 1;
      continue;
    }
    if (lines.length === maxLines - 1) return [...lines, ellipsized(line, maxWidth, measure)];
    // 一个词写到一半放不下：退回上一个空格，整个词挪到下一行。
    const space = character === ' ' ? -1 : line.lastIndexOf(' ');
    if (space > 0) {
      index -= Array.from(line.slice(space + 1)).length;
      line = line.slice(0, space);
    }
    lines.push(line.trimEnd());
    line = '';
  }
  return line === '' ? lines : [...lines, line.trimEnd()];
}

function ellipsized(line: string, maxWidth: number, measure: (text: string) => number): string {
  const characters = Array.from(line.trimEnd());
  while (characters.length > 1 && measure(`${characters.join('')}…`) > maxWidth) characters.pop();
  return `${characters.join('').trimEnd()}…`;
}
