import type { SceneCharacter } from './scene-character';
import { estimatedTextWidth, labelStack, nameplate, ownerTag, statusSticker } from './scene-style';

const segments = new Intl.Segmenter(undefined, { granularity: 'grapheme' });
const graphemes = (text: string) => Array.from(segments.segment(text), (entry) => entry.segment);
/** Agent 名字后面的四位编号（specs/agent-name-tag/design.md），同名的靠它分开。 */
const nameTag = / #\d{4}$/u;

export function characterLabel(name: string, limit = 20): string {
  const trimmed = name.trim();
  const letters = graphemes(trimmed);
  if (letters.length <= limit) return trimmed;
  // 太长时截前面的名字、留着编号，不然同名的又分不开了。
  const tag = nameTag.exec(trimmed)?.[0];
  if (tag !== undefined && limit > tag.length + 1) {
    const base = graphemes(trimmed.slice(0, -tag.length).trimEnd());
    return `${base
      .slice(0, limit - tag.length - 1)
      .join('')
      .trimEnd()}…${tag}`;
  }
  return `${letters.slice(0, limit - 1).join('')}…`;
}

/**
 * 名牌下“谁的 Agent”那一行最多这么长。里面的人名已经按 {@link OWNER_NAME_LIMIT} 截过，
 * 这里只兜底，免得把“放行”之类的后半句截掉。
 */
export const OWNER_LABEL_LIMIT = 40;
/** “谁的 Agent”里的人名最多这么长。 */
export const OWNER_NAME_LIMIT = 16;

type Rect = {
  readonly x: number;
  readonly y: number;
  readonly width: number;
  readonly height: number;
};
const overlaps = (a: Rect, b: Rect) =>
  a.x < b.x + b.width && a.x + a.width > b.x && a.y < b.y + b.height && a.y + a.height > b.y;

/** 人物会在落点附近走动；每侧留出这段余量，走近时名牌不至于立刻互相压住。 */
const roamAllowance = 24;

/** Prioritize the selected person, then self, then stable IDs. Use a spatial
 * index so label selection does not become quadratic in a crowded room.
 * The reserved box is the drawn nameplate (or status sticker, when wider) plus
 * roaming room — the same width estimate that sizes the pills themselves. */
export function visibleCharacterLabels(
  characters: readonly SceneCharacter[],
  selectedId: string | null,
  near: boolean,
  statusLabel?: (node: SceneCharacter) => string | undefined,
): ReadonlySet<string> {
  const visible = new Set<string>();
  const cells = new Map<string, Rect[]>();
  const keys = (box: Rect) => {
    const result: string[] = [];
    for (let x = Math.floor(box.x / 160); x <= Math.floor((box.x + box.width) / 160); x += 1)
      for (let y = Math.floor(box.y / 160); y <= Math.floor((box.y + box.height) / 160); y += 1)
        result.push(`${String(x)}:${String(y)}`);
    return result;
  };
  const priority = (node: SceneCharacter) =>
    node.characterId === selectedId ? 0 : node.isSelf ? 1 : 2;
  const candidates = characters
    .filter((node) => near || node.kind === 'human' || node.characterId === selectedId)
    .toSorted(
      (a, b) =>
        priority(a) - priority(b) ||
        (a.characterId < b.characterId ? -1 : a.characterId > b.characterId ? 1 : 0),
    );
  for (const node of candidates) {
    const size = Math.max(0.83, node.radius / 27);
    const plate =
      estimatedTextWidth(characterLabel(node.displayName), nameplate.fontSize) +
      nameplate.paddingX * 2;
    const status = near && node.kind === 'agent' ? statusLabel?.(node) : undefined;
    const sticker =
      status === undefined
        ? 0
        : estimatedTextWidth(status, statusSticker.fontSize) + statusSticker.paddingX * 2;
    const owner =
      near && node.kind === 'agent' && node.ownerLabel !== undefined
        ? characterLabel(node.ownerLabel, OWNER_LABEL_LIMIT)
        : undefined;
    const ownerWidth =
      owner === undefined
        ? 0
        : estimatedTextWidth(owner, ownerTag.fontSize) + ownerTag.paddingX * 2;
    // 名牌下还写着“谁的 Agent”时，往下多留那一行。
    const bottom =
      owner === undefined
        ? 66
        : Math.max(66, labelStack(status !== undefined).owner + ownerTag.height + 2);
    const width = (Math.max(plate, sticker, ownerWidth) + roamAllowance * 2) * size;
    const box = { x: node.x - width / 2, y: node.y + 8 * size, width, height: (bottom - 8) * size };
    const buckets = keys(box);
    if (buckets.some((key) => cells.get(key)?.some((other) => overlaps(box, other)))) continue;
    visible.add(node.characterId);
    for (const key of buckets) {
      const bucket = cells.get(key) ?? [];
      bucket.push(box);
      cells.set(key, bucket);
    }
  }
  return visible;
}
