const graphemes = new Intl.Segmenter(undefined, { granularity: 'grapheme' });

/** 头像里的那一个字：名字的第一个完整字素。名字是空的时给一个问号，圆里不留白。 */
export function initial(displayName: string): string {
  for (const { segment } of graphemes.segment(displayName.trim())) return segment.toUpperCase();
  return '?';
}
