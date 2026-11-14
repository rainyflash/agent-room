/**
 * 聊天正文用的受限 Markdown：只认代码块、行内代码、粗体、标题、列表、引用、管道线写的表格，
 * 和直接写出来的 `http`、`https` 地址。HTML、图片和 `[字](地址)` 这种写法一律当普通文字。
 * 地址只是标出来：渲染层不生成能直接打开的链接，点了要先确认（见 `ui/external-link.tsx`）。
 */

export type MarkdownInline =
  | { readonly kind: 'text'; readonly text: string }
  | { readonly kind: 'code'; readonly text: string }
  | { readonly kind: 'strong'; readonly text: string }
  /** `text` 是正文里写的样子，`href` 是浏览器认的写法：确认框里写它，打开的也是它。 */
  | { readonly kind: 'link'; readonly text: string; readonly href: string };

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
/**
 * 正文里直接写出来的地址。只认 ASCII 里地址用得到的那些字符：中文里地址后面常常直接跟着字
 * （“打开https://example.com看看”），认别的字符的话后面的字会被吞进地址里。域名或者路径里带
 * 别的字符的地址因此只认到那个字符为止，要点开整个地址得写成转码以后的样子。
 * 字符一个个写出来，别换成 `\w` 加 `i`：两样一起用时 `\w` 还认两个不是 ASCII 的字母。
 * 方括号不在里面：`[字](地址)` 这种写法里，字要是也写成地址，认到 `]` 就得停。
 */
const linkPattern = /[Hh][Tt][Tt][Pp][Ss]?:\/\/[A-Za-z0-9_\-.~:/?#@!$&'()*+,;=%]+/gu;
/** 地址后面跟着的标点多半是句子的，不是地址的。 */
const trailingPunctuation = new Set(['.', ',', ';', ':', '!', '?', "'", '*', '_', '~']);
/** 再长的地址不标成能点的：确认框里看不过来，也多半不是给人点的。 */
const maxLinkLength = 2048;
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
    .flatMap((piece): readonly MarkdownInline[] => {
      if (piece.startsWith('`') && piece.endsWith('`') && piece.length > 2) {
        return [{ kind: 'code', text: piece.slice(1, -1) }];
      }
      if (piece.startsWith('**') && piece.endsWith('**') && piece.length > 4) {
        return [{ kind: 'strong', text: piece.slice(2, -2) }];
      }
      return withLinks(piece);
    });
}

/** 一段普通的字里直接写出来的地址拆出来，别的照旧是字。 */
function withLinks(text: string): readonly MarkdownInline[] {
  const pieces: MarkdownInline[] = [];
  let from = 0;
  for (const match of text.matchAll(linkPattern)) {
    const written = withoutTrailingPunctuation(match[0]);
    const href = linkTarget(written);
    if (href === null) continue;
    if (match.index > from) pieces.push({ kind: 'text', text: text.slice(from, match.index) });
    pieces.push({ kind: 'link', text: written, href });
    from = match.index + written.length;
  }
  if (from < text.length) pieces.push({ kind: 'text', text: text.slice(from) });
  return pieces;
}

/** 去掉末尾属于句子的标点；右括号只在落单时去掉，`…/wiki/A_(B)` 这样成对的留着。 */
function withoutTrailingPunctuation(candidate: string): string {
  let end = candidate.length;
  while (end > 0) {
    const last = candidate[end - 1] ?? '';
    const kept = candidate.slice(0, end);
    if (trailingPunctuation.has(last)) end -= 1;
    else if (last === ')' && count(kept, ')') > count(kept, '(')) end -= 1;
    else break;
  }
  return candidate.slice(0, end);
}

function count(text: string, char: string): number {
  return text.split(char).length - 1;
}

/**
 * 这个地址能不能标成能点的，能的话回浏览器认的写法。只认 `http`、`https`；带用户名、密码的不认：
 * `https://认识的网站@别的网站/` 这种写法就是拿来骗人看错域名的。
 */
function linkTarget(written: string): string | null {
  if (written.length > maxLinkLength) return null;
  let url: URL;
  try {
    url = new URL(written);
  } catch {
    return null;
  }
  if (url.protocol !== 'http:' && url.protocol !== 'https:') return null;
  if (url.hostname.length === 0 || url.username.length > 0 || url.password.length > 0) return null;
  return url.href;
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
