import { encode } from 'uqr';

import { loadStudioCharacters } from '@/features/lobby/scene/studio-assets';
import {
  SHARE_CARD_QR,
  SHARE_CARD_SIZE,
  shareCardQrLayout,
  shareCardSite,
  wrapCardText,
  type ShareCardContent,
} from '@/features/room-share/domain/share-card';
import type { RoomTone } from '@/shared/ui/room-tone';

/**
 * 把卡片画成一张 PNG（`specs/share-card/design.md`）。在这台设备上画：邀请链接的秘密在 `#` 后面，
 * 不该为了一张图交给服务器。颜色一律用白天的那一套：图是发给别人看的，不跟着这台设备的夜间走。
 */
const SANS = "'Geist Variable', 'Noto Sans SC Variable', sans-serif";

/**
 * 和 `packages/ui-system/src/styles.css` 白天的令牌一样；画布里用不了变量，只能抄一份。
 * `share-card-canvas.spec.ts` 卡着两边一样：改令牌时它会红，跟着改这里。
 */
export const SHARE_CARD_COLORS = Object.freeze({
  '--color-ink': '#18181b',
  '--ar-muted': '#686873',
  '--ar-canvas': '#ffffff',
  '--ar-surface': '#f6f8f8',
  '--ar-nav': '#12302e',
  '--ar-nav-ink': '#eef6f4',
  '--ar-tone-teal': '#0b7268',
  '--ar-tone-coral': '#b83a25',
  '--ar-tone-sunny': '#8a5708',
  '--ar-tone-sky': '#1d5fa8',
  '--ar-tint-teal': '#c5efe5',
  '--ar-tint-coral': '#ffdfd6',
  '--ar-tint-sunny': '#ffe494',
  '--ar-tint-sky': '#d3e6fb',
});
const INK = SHARE_CARD_COLORS['--color-ink'];
const MUTED = SHARE_CARD_COLORS['--ar-muted'];
const PAPER = SHARE_CARD_COLORS['--ar-canvas'];
const SURFACE = SHARE_CARD_COLORS['--ar-surface'];
const NAV = SHARE_CARD_COLORS['--ar-nav'];
const NAV_INK = SHARE_CARD_COLORS['--ar-nav-ink'];
/** 标志自己的两种颜色，和 `public/agent-room-mark.svg` 一样。 */
const MARK = '#247a77';
const MARK_DOT = '#bde8d8';
const TONES: Readonly<Record<RoomTone, { readonly ink: string; readonly tint: string }>> = {
  teal: { ink: SHARE_CARD_COLORS['--ar-tone-teal'], tint: SHARE_CARD_COLORS['--ar-tint-teal'] },
  coral: { ink: SHARE_CARD_COLORS['--ar-tone-coral'], tint: SHARE_CARD_COLORS['--ar-tint-coral'] },
  sunny: { ink: SHARE_CARD_COLORS['--ar-tone-sunny'], tint: SHARE_CARD_COLORS['--ar-tint-sunny'] },
  sky: { ink: SHARE_CARD_COLORS['--ar-tone-sky'], tint: SHARE_CARD_COLORS['--ar-tint-sky'] },
};

const LEFT = 80;
const RIGHT = SHARE_CARD_SIZE.width - 80;
const BAND_HEIGHT = 160;
const TILE = { x: LEFT, y: 312, size: 96 };
const PANEL = { x: LEFT, y: 500, width: RIGHT - LEFT, height: 550 };
const PANEL_TEXT = { x: SHARE_CARD_QR.x + SHARE_CARD_QR.size + 50, width: 325 };

/** 标志里那条线，和 `public/agent-room-mark.svg` 一样（64 见方）。 */
const MARK_PATH = 'M33 48H24a8 8 0 0 1-8-8V24a8 8 0 0 1 8-8h16a8 8 0 0 1 8 8v9';
/** 房间小图标里的门（24 见方），和“房间”页卡片上那一个一样。 */
const DOOR_PATHS = [
  'M11 20H2',
  'M11 4.562v16.157a1 1 0 0 0 1.242.97L19 20V5.562a2 2 0 0 0-1.515-1.94l-4-1A2 2 0 0 0 11 4.561z',
  'M11 4H8a2 2 0 0 0-2 2v14',
  'M14 12h.01',
  'M22 20h-3',
] as const;

/**
 * 左下角的三个机器人：站着的那一排里的方的、圆的、胶囊的。`source` 是它在图集里占的那一块，
 * `circle` 是垫在底下的浅色圆。
 */
const ROBOTS = [
  {
    source: { x: 92, y: 162, width: 252, height: 286 },
    circle: { x: 164, y: 1196, radius: 84, fill: '#e2f3ef' },
  },
  {
    source: { x: 487, y: 177, width: 206, height: 281 },
    circle: { x: 330, y: 1216, radius: 64, fill: '#fde9e5' },
  },
  {
    source: { x: 842, y: 180, width: 198, height: 278 },
    circle: { x: 468, y: 1196, radius: 58, fill: '#fff2cc' },
  },
] as const;

const FONT_WAIT_MS = 3_000;

export async function renderShareCard(content: ShareCardContent): Promise<Blob> {
  const canvas = document.createElement('canvas');
  canvas.width = SHARE_CARD_SIZE.width;
  canvas.height = SHARE_CARD_SIZE.height;
  const context = canvas.getContext('2d');
  if (context === null) throw new Error('Share card requires a 2D canvas.');
  const [robots] = await Promise.all([
    // 机器人只是点缀：图集要不到就不画，卡片照出。
    loadStudioCharacters().catch(() => null),
    fontsReady(content),
  ]);
  drawBackground(context);
  drawBand(context);
  drawRoom(context, content);
  drawPanel(context, content);
  drawFooter(context, content, robots);
  return await new Promise<Blob>((resolve, reject) => {
    canvas.toBlob((blob) => {
      if (blob === null) reject(new Error('Share card could not be encoded.'));
      else resolve(blob);
    }, 'image/png');
  });
}

/**
 * 画之前把要用的字要齐：中文字体是按字分块的，没用过的字要现下。等不到（离线、太慢）就用系统的字画，
 * 不为这个卡住。
 */
async function fontsReady(content: ShareCardContent): Promise<void> {
  const text = [
    'Agent Room',
    content.lead,
    content.roomName,
    content.action,
    content.tagline,
    ...content.notes,
    shareCardSite(content.url),
  ].join(' ');
  const wanted = Promise.all(
    [500, 600, 700].map((weight) => document.fonts.load(`${String(weight)} 40px ${SANS}`, text)),
  );
  const giveUp = new Promise<void>((resolve) => {
    globalThis.setTimeout(resolve, FONT_WAIT_MS);
  });
  await Promise.race([wanted, giveUp]).catch(() => undefined);
}

function font(weight: number, size: number): string {
  return `${String(weight)} ${String(size)}px ${SANS}`;
}

function drawBackground(context: CanvasRenderingContext2D): void {
  context.fillStyle = PAPER;
  context.fillRect(0, 0, SHARE_CARD_SIZE.width, SHARE_CARD_SIZE.height);
}

function drawBand(context: CanvasRenderingContext2D): void {
  context.fillStyle = NAV;
  context.fillRect(0, 0, SHARE_CARD_SIZE.width, BAND_HEIGHT);
  const top = (BAND_HEIGHT - 64) / 2;
  context.save();
  context.translate(LEFT, top);
  context.fillStyle = MARK;
  roundedRect(context, 0, 0, 64, 64, 16);
  context.fill();
  context.strokeStyle = PAPER;
  context.lineWidth = 5;
  context.lineCap = 'round';
  context.lineJoin = 'round';
  context.stroke(new Path2D(MARK_PATH));
  dot(context, 27, 28, 4.5, PAPER);
  dot(context, 39, 40, 4.5, MARK_DOT);
  context.restore();
  context.fillStyle = NAV_INK;
  context.font = font(600, 38);
  context.textAlign = 'left';
  context.textBaseline = 'middle';
  context.fillText('Agent Room', LEFT + 64 + 20, BAND_HEIGHT / 2 + 2);
}

function drawRoom(context: CanvasRenderingContext2D, content: ShareCardContent): void {
  context.textAlign = 'left';
  context.textBaseline = 'alphabetic';
  context.fillStyle = MUTED;
  context.font = font(500, 36);
  const lead = wrapCardText(content.lead, RIGHT - LEFT, 1, measureWith(context));
  context.fillText(lead[0] ?? '', LEFT, 268);

  const tone = TONES[content.tone];
  context.fillStyle = tone.tint;
  roundedRect(context, TILE.x, TILE.y, TILE.size, TILE.size, 24);
  context.fill();
  context.save();
  // 门的图是 24 见方，放大到 54，摆在小方块正中。
  context.translate(TILE.x + (TILE.size - 54) / 2, TILE.y + (TILE.size - 54) / 2);
  context.scale(2.25, 2.25);
  context.strokeStyle = tone.ink;
  context.lineWidth = 2;
  context.lineCap = 'round';
  context.lineJoin = 'round';
  for (const path of DOOR_PATHS) context.stroke(new Path2D(path));
  context.restore();

  const nameLeft = TILE.x + TILE.size + 28;
  context.fillStyle = INK;
  context.font = font(700, 72);
  const lines = wrapCardText(content.roomName, RIGHT - nameLeft, 2, measureWith(context));
  const middle = TILE.y + TILE.size / 2;
  lines.forEach((line, index) => {
    context.fillText(line, nameLeft, middle + (index - (lines.length - 1) / 2) * 86 + 25);
  });
}

function drawPanel(context: CanvasRenderingContext2D, content: ShareCardContent): void {
  context.fillStyle = SURFACE;
  roundedRect(context, PANEL.x, PANEL.y, PANEL.width, PANEL.height, 40);
  context.fill();
  context.fillStyle = PAPER;
  roundedRect(
    context,
    SHARE_CARD_QR.x,
    SHARE_CARD_QR.y,
    SHARE_CARD_QR.size,
    SHARE_CARD_QR.size,
    28,
  );
  context.fill();
  drawQrCode(context, content.url);

  context.textAlign = 'left';
  context.textBaseline = 'alphabetic';
  context.font = font(700, 48);
  const action = wrapCardText(content.action, PANEL_TEXT.width, 2, measureWith(context));
  context.font = font(500, 28);
  const notes = content.notes.flatMap((note) =>
    wrapCardText(note, PANEL_TEXT.width, 2, measureWith(context)),
  );
  const height = action.length * 60 + (notes.length === 0 ? 0 : 16 + notes.length * 42);
  let baseline = PANEL.y + (PANEL.height - height) / 2 + 46;
  context.fillStyle = INK;
  context.font = font(700, 48);
  for (const line of action) {
    context.fillText(line, PANEL_TEXT.x, baseline);
    baseline += 60;
  }
  baseline += 16 - 60 + 42;
  context.fillStyle = MUTED;
  context.font = font(500, 28);
  for (const line of notes) {
    context.fillText(line, PANEL_TEXT.x, baseline);
    baseline += 42;
  }
}

/** 二维码：白底黑块，扫码的软件认这个。 */
function drawQrCode(context: CanvasRenderingContext2D, url: string): void {
  const qr = encode(url, { border: 0, ecc: 'M' });
  const layout = shareCardQrLayout(qr.size);
  context.fillStyle = '#000000';
  qr.data.forEach((row, y) => {
    row.forEach((dark, x) => {
      if (!dark) return;
      context.fillRect(
        layout.x + x * layout.cell,
        layout.y + y * layout.cell,
        layout.cell,
        layout.cell,
      );
    });
  });
}

function drawFooter(
  context: CanvasRenderingContext2D,
  content: ShareCardContent,
  robots: HTMLCanvasElement | null,
): void {
  if (robots !== null) {
    for (const robot of ROBOTS) {
      dot(context, robot.circle.x, robot.circle.y, robot.circle.radius, robot.circle.fill);
      // 机器人站在圆里：高是圆的一倍半左右，宽照它自己的比例。
      const height = robot.circle.radius * 1.5;
      const width = (height * robot.source.width) / robot.source.height;
      context.drawImage(
        robots,
        robot.source.x,
        robot.source.y,
        robot.source.width,
        robot.source.height,
        robot.circle.x - width / 2,
        robot.circle.y - height / 2,
        width,
        height,
      );
    }
  }
  const textLeft = robots === null ? LEFT : 560;
  context.textAlign = 'right';
  context.textBaseline = 'alphabetic';
  context.font = font(500, 28);
  const tagline = wrapCardText(content.tagline, RIGHT - textLeft, 2, measureWith(context));
  const site = shareCardSite(content.url);
  // 最下面一行是网址，那一句话在它上面，往上排。
  let baseline = 1270 - (site === '' ? 0 : 44) - (tagline.length - 1) * 40;
  context.fillStyle = INK;
  for (const line of tagline) {
    context.fillText(line, RIGHT, baseline);
    baseline += 40;
  }
  if (site === '') return;
  context.fillStyle = MUTED;
  context.font = font(500, 26);
  const shown = wrapCardText(site, RIGHT - textLeft, 1, measureWith(context));
  context.fillText(shown[0] ?? '', RIGHT, 1270);
}

function measureWith(context: CanvasRenderingContext2D): (text: string) => number {
  return (text) => context.measureText(text).width;
}

function dot(
  context: CanvasRenderingContext2D,
  x: number,
  y: number,
  radius: number,
  fill: string,
): void {
  context.fillStyle = fill;
  context.beginPath();
  context.arc(x, y, radius, 0, Math.PI * 2);
  context.fill();
}

/** `roundRect` 旧一点的浏览器没有（iOS 15），自己描。 */
function roundedRect(
  context: CanvasRenderingContext2D,
  x: number,
  y: number,
  width: number,
  height: number,
  radius: number,
): void {
  context.beginPath();
  context.moveTo(x + radius, y);
  context.arcTo(x + width, y, x + width, y + height, radius);
  context.arcTo(x + width, y + height, x, y + height, radius);
  context.arcTo(x, y + height, x, y, radius);
  context.arcTo(x, y, x + width, y, radius);
  context.closePath();
}
