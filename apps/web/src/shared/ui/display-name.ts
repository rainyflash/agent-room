const graphemes = new Intl.Segmenter(undefined, { granularity: 'grapheme' });

/** 取两个完整字素作为头像缩写，避免拆开组合字符和表情。 */
export function initials(displayName: string): string {
  let result = '';
  let count = 0;
  for (const { segment } of graphemes.segment(displayName.trim())) {
    result += segment;
    if (++count === 2) break;
  }
  return result.toUpperCase();
}
