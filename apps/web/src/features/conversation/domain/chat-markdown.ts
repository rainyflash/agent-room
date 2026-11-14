/**
 * 聊天正文用的受限 Markdown：只认代码块、行内代码、粗体、标题、列表、引用和管道线写的表格。
 * HTML 与链接一律当普通文字，渲染层不会生成任何可点击或可执行的节点。
 */

export type MarkdownInline =
  | { readonly kind: 'text'; readonly text: string }
  | { readonly kind: 'code'; readonly text: string }
  | { readonly kind: 'strong'; readonly text: string };

export type MarkdownAlign = 'left' | 'center' | 'right' | null;
export type MarkdownCell = readonly MarkdownInline[];

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
  | {
      readonly kind: 'table';
      /** 每一列往哪边靠；分隔那一行没写冒号就是 `null`，照默认的来。 */
      readonly align: readonly MarkdownAlign[];
      readonly header: readonly MarkdownCell[];
      /** 每一行的格子数和表头一样：少的补空，多的不要。 */
      readonly rows: readonly (readonly MarkdownCell[])[];
    }
  | { readonly kind: 'quote'; readonly lines: readonly (readonly MarkdownInline[])[] }
  | { readonly kind: 'paragraph'; readonly lines: readonly (readonly MarkdownInline[])[] };

const fencePattern = /^\s*```\s*([\w+#.-]{0,32})\s*$/u;
const headingPattern = /^(#{1,3})\s+(.*)$/u;
const bulletPattern = /^\s*[-*+]\s+(.*)$/u;
const orderedPattern = /^\s*\d{1,3}[.)]\s+(.*)$/u;
const quotePattern = /^>\s?(.*)$/u;
const inlinePattern = /(`[^`\n]+`|\*\*[^*\n]+\*\*)/u;
const tableDelimiterPattern = /^:?-+:?$/u;
/** 一张表最多几列。再多的不当表格，照原样显示：多半不是表格，是碰巧带着竖线的字。 */
const maxTableColumns = 16;

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
    const align = tableAlignment(lines, index);
    if (align !== null) {
      const header = tableCells(line) ?? [];
      const rows: (readonly MarkdownCell[])[] = [];
      index += 2;
      while (index < lines.length) {
        const current = lines[index] ?? '';
        const cells = fencePattern.test(current) ? null : tableCells(current);
        if (cells === null) break;
        rows.push(align.map((_, column) => parseInline(cells[column] ?? '')));
        index += 1;
      }
      blocks.push({ kind: 'table', align, header: header.map(parseInline), rows });
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
        quotePattern.test(current) ||
        tableAlignment(lines, index) !== null
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

/** 这一行按竖线拆成的格子；不带竖线、是空行就不是表格的一行。反斜杠加竖线是格子里的一条竖线。 */
function tableCells(line: string): string[] | null {
  let text = line.trim();
  if (!text.includes('|')) return null;
  if (text.startsWith('|')) text = text.slice(1);
  if (text.endsWith('|') && !text.endsWith('\\|')) text = text.slice(0, -1);
  const cells: string[] = [];
  let cell = '';
  for (let at = 0; at < text.length; at += 1) {
    const char = text[at] ?? '';
    if (char === '\\' && text[at + 1] === '|') {
      cell += '|';
      at += 1;
    } else if (char === '|') {
      cells.push(cell.trim());
      cell = '';
    } else cell += char;
  }
  cells.push(cell.trim());
  return cells;
}

/**
 * 从这一行起是不是一张表：这一行是表头，下一行是 `--- | :---:` 这样的分隔，两行格子数一样。
 * 是的话回每一列往哪边靠。
 */
function tableAlignment(lines: readonly string[], index: number): MarkdownAlign[] | null {
  const header = tableCells(lines[index] ?? '');
  const delimiter = tableCells(lines[index + 1] ?? '');
  if (header === null || delimiter === null) return null;
  if (header.length !== delimiter.length || header.length > maxTableColumns) return null;
  if (!delimiter.every((cell) => tableDelimiterPattern.test(cell))) return null;
  return delimiter.map((cell) => {
    const left = cell.startsWith(':');
    const right = cell.endsWith(':');
    return left && right ? 'center' : right ? 'right' : left ? 'left' : null;
  });
}

/** 只要字：头顶气泡这种一行的地方不渲染 Markdown，标记符号也别露出来。 */
export function chatMarkdownPlainText(source: string): string {
  return parseChatMarkdown(source)
    .map((block) => {
      switch (block.kind) {
        case 'code':
          return block.text;
        case 'heading':
          return inlineText(block.inline);
        case 'list':
          return block.items.map(inlineText).join('\n');
        case 'table':
          return [block.header, ...block.rows]
            .map((row) => row.map(inlineText).join('  '))
            .join('\n');
        case 'quote':
        case 'paragraph':
          return block.lines.map(inlineText).join('\n');
      }
    })
    .join('\n');
}

function inlineText(inline: readonly MarkdownInline[]): string {
  return inline.map((piece) => piece.text).join('');
}
