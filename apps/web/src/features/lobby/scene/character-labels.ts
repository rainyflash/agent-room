import type { SceneCharacter } from './scene-character';

const segments = new Intl.Segmenter(undefined, { granularity: 'grapheme' });
export function characterLabel(name: string): string {
  const letters = Array.from(segments.segment(name.trim()), (entry) => entry.segment);
  return letters.length <= 20 ? name.trim() : `${letters.slice(0, 19).join('')}…`;
}

type Rect = {
  readonly x: number;
  readonly y: number;
  readonly width: number;
  readonly height: number;
};
const overlaps = (a: Rect, b: Rect) =>
  a.x < b.x + b.width && a.x + a.width > b.x && a.y < b.y + b.height && a.y + a.height > b.y;

/** Prioritize the selected person, then self, then stable IDs. Use a spatial
 * index so label selection does not become quadratic in a crowded room. */
export function visibleCharacterLabels(
  characters: readonly SceneCharacter[],
  selectedId: string | null,
  near: boolean,
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
    // Bound using the widest glyph, including CJK and emoji; reserve motion room.
    const width =
      (Array.from(segments.segment(characterLabel(node.displayName))).length * 18 + 24) * size;
    const box = { x: node.x - width / 2, y: node.y + 8 * size, width, height: 58 * size };
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
