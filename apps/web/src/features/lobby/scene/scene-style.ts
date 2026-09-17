import type { LobbyAgentStatus } from '@/features/lobby/domain/lobby';

/**
 * 游戏大厅场景的颜色、描边与名牌几何。Pixi 与无 WebGL 的 SVG 回退共用这一份，
 * 以前两边各写一套字面量，选中环、徽标描边和名牌位置已经悄悄不一致。
 * 画布读不到 CSS 变量，这里的颜色与 packages/ui-system 的令牌保持同值。
 */
export const sceneInk = '#2a2733';
export const sceneFont = 'Fredoka Variable, Noto Sans SC Variable, sans-serif';

export const sceneStrokeWidth = { wall: 6, furniture: 3 } as const;

export const sceneFloor = { tile: 56, light: '#fff1d3', dark: '#fbe6bc' } as const;

export const characterShadow = { color: sceneInk, alpha: 0.16 } as const;

export const selectionRing = {
  outer: sceneInk,
  outerWidth: 6,
  inner: '#ffc53d',
  innerWidth: 3,
} as const;

export const statusBadgeFill = { waiting_input: '#ffb199', blocked: '#ffd27a' } as const;

export const humanMarker = { fill: sceneInk, mark: '#ffffff' } as const;

/** 状态贴纸与头顶圆点共用的底色：彩色底上只放墨色字，对比度都在 8:1 以上。 */
export const statusStickerFill: Readonly<Record<LobbyAgentStatus | 'present', string>> = {
  present: '#ffffff',
  working: '#bde8d8',
  idle: '#ffffff',
  completed: '#bde8d8',
  waiting_input: '#ffb199',
  blocked: '#ffd27a',
  offline: '#e9e4da',
};

/** 名牌在人物脚下；标签避让在 character-labels.ts 里预留了 y + 8 到 y + 66 的高度。 */
export const nameplate = {
  top: 12,
  height: 26,
  paddingX: 10,
  fontSize: 15,
  fontWeight: '600',
  fill: '#ffffff',
  stroke: sceneInk,
  strokeWidth: 2,
} as const;

export const statusSticker = {
  gap: 4,
  height: 22,
  paddingX: 8,
  fontSize: 13,
  fontWeight: '600',
  stroke: sceneInk,
  strokeWidth: 2,
} as const;

const segments = new Intl.Segmenter(undefined, { granularity: 'grapheme' });

/** SVG 回退拿不到画布测量，按字形估宽：中日韩与表情占满一个字号，其余约 0.56 个字号。 */
export function estimatedTextWidth(text: string, fontSize: number): number {
  let width = 0;
  for (const { segment } of segments.segment(text)) {
    const codePoint = segment.codePointAt(0) ?? 0;
    width += codePoint >= 0x2e80 ? fontSize : fontSize * 0.56;
  }
  return Math.ceil(width);
}
