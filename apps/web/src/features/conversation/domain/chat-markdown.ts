/**
 * 聊天正文用的受限 Markdown：只认代码块、行内代码、粗体、标题、列表和引用。
 * HTML 与链接一律当普通文字，渲染层不会生成任何可点击或可执行的节点。
 */

export type MarkdownInline =
  | { readonly kind: 'text'; readonly text: string }
  | { readonly kind: 'code'; readonly text: string }
  | { readonly kind: 'strong'; readonly text: string };

export type MarkdownBlock =
  | { readonly kind: 'code'; readonly language: string; readonly text: string }
  | {
      readonly kind: 'heading';
      readonly level: 1 | 2 | 3;
      readonly inline: readonly MarkdownInline[];
    }
  | {
      readonly kind: 'list';
      readonly ordered: boolean;
      readonly items: readonly (readonly MarkdownInline[])[];
    }
  | { readonly kind: 'quote'; readonly lines: readonly (readonly MarkdownInline[])[] }
  | { readonly kind: 'paragraph'; readonly lines: readonly (readonly MarkdownInline[])[] };

const fencePattern = /^\s*```\s*([\w+#.-]{0,32})\s*$/u;
const headingPattern = /^(#{1,3})\s+(.*)$/u;
const bulletPattern = /^\s*[-*+]\s+(.*)$/u;
const orderedPattern = /^\s*\d{1,3}[.)]\s+(.*)$/u;
const quotePattern = /^>\s?(.*)$/u;
const inlinePattern = /(`[^`\n]+`|\*\*[^*\n]+\*\*)/u;

export function parseChatMarkdown(source: string): readonly MarkdownBlock[] {
  const blocks: MarkdownBlock[] = [];
  const lines = source.split(/\r?\n/u);
  let index = 0;
  while (index < lines.length) {
    const line = lines[index] ?? '';
    const fence = fencePattern.exec(line);
    if (fence) {
      const language = fence[1] ?? '';
      const body: string[] = [];
      index += 1;
      while (index < lines.length && !/^\s*```\s*$/u.test(lines[index] ?? '')) {
        body.push(lines[index] ?? '');
        index += 1;
      }
      index += 1;
      blocks.push({ kind: 'code', language, text: body.join('\n') });
      continue;
    }
    if (line.trim().length === 0) {
      index += 1;
      continue;
    }
    const heading = headingPattern.exec(line);
    if (heading) {
      const level = (heading[1]?.length ?? 1) as 1 | 2 | 3;
      blocks.push({ kind: 'heading', level, inline: parseInline(heading[2] ?? '') });
      index += 1;
      continue;
    }
    if (bulletPattern.test(line) || orderedPattern.test(line)) {
      const ordered = orderedPattern.test(line);
      const pattern = ordered ? orderedPattern : bulletPattern;
      const items: (readonly MarkdownInline[])[] = [];
      while (index < lines.length) {
        const item = pattern.exec(lines[index] ?? '');
        if (!item) break;
        items.push(parseInline(item[1] ?? ''));
        index += 1;
      }
      blocks.push({ kind: 'list', ordered, items });
      continue;
    }
    if (quotePattern.test(line)) {
      const quoted: (readonly MarkdownInline[])[] = [];
      while (index < lines.length) {
        const quote = quotePattern.exec(lines[index] ?? '');
        if (!quote) break;
        quoted.push(parseInline(quote[1] ?? ''));
        index += 1;
      }
      blocks.push({ kind: 'quote', lines: quoted });
      continue;
    }
    const paragraph: (readonly MarkdownInline[])[] = [];
    while (index < lines.length) {
      const current = lines[index] ?? '';
      if (
        current.trim().length === 0 ||
        fencePattern.test(current) ||
        headingPattern.test(current) ||
        bulletPattern.test(current) ||
        orderedPattern.test(current) ||
        quotePattern.test(current)
      ) {
        break;
      }
      paragraph.push(parseInline(current));
      index += 1;
    }
    blocks.push({ kind: 'paragraph', lines: paragraph });
  }
  return blocks;
}

export function parseInline(text: string): readonly MarkdownInline[] {
  return text
    .split(inlinePattern)
    .filter((piece) => piece.length > 0)
    .map((piece) => {
      if (piece.startsWith('`') && piece.endsWith('`') && piece.length > 2) {
        return { kind: 'code', text: piece.slice(1, -1) } as const;
      }
      if (piece.startsWith('**') && piece.endsWith('**') && piece.length > 4) {
        return { kind: 'strong', text: piece.slice(2, -2) } as const;
      }
      return { kind: 'text', text: piece } as const;
    });
}
