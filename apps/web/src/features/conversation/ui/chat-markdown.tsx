import { Fragment } from 'react';
import {
  parseChatMarkdown,
  type MarkdownBlock,
  type MarkdownInline,
} from '@/features/conversation/domain/chat-markdown';

/**
 * 聊天气泡里的正文：Agent 的回复常带代码块和列表，纯文字显示读不了。
 * 只产生文字、粗体、行内代码、代码块、列表和引用节点，永远没有链接、图片或 HTML。
 */
export function ChatMarkdown({ source }: { readonly source: string }) {
  return (
    <div className="chat-markdown">
      {parseChatMarkdown(source).map((block, index) => renderBlock(block, index))}
    </div>
  );
}

function renderBlock(block: MarkdownBlock, index: number) {
  const key = `${String(index)}-${block.kind}`;
  switch (block.kind) {
    case 'code':
      return (
        <pre className="chat-markdown__code" data-language={block.language || undefined} key={key}>
          <code>{block.text}</code>
        </pre>
      );
    case 'heading':
      // 气泡里的标题只是加粗的一行，不进文档大纲。
      return (
        <p className="chat-markdown__heading" data-level={block.level} key={key}>
          {renderInline(block.inline)}
        </p>
      );
    case 'list': {
      const items = block.items.map((item, itemIndex) => (
        <li key={`${key}-${String(itemIndex)}`}>{renderInline(item)}</li>
      ));
      return block.ordered ? <ol key={key}>{items}</ol> : <ul key={key}>{items}</ul>;
    }
    case 'quote':
      return (
        <blockquote className="chat-markdown__quote" key={key}>
          {renderLines(block.lines)}
        </blockquote>
      );
    case 'paragraph':
      return <p key={key}>{renderLines(block.lines)}</p>;
  }
}

function renderLines(lines: readonly (readonly MarkdownInline[])[]) {
  return lines.map((line, index) => (
    <Fragment key={String(index)}>
      {index > 0 ? <br /> : null}
      {renderInline(line)}
    </Fragment>
  ));
}

function renderInline(inline: readonly MarkdownInline[]) {
  return inline.map((piece, index) => {
    const key = String(index);
    switch (piece.kind) {
      case 'code':
        return <code key={key}>{piece.text}</code>;
      case 'strong':
        return <strong key={key}>{piece.text}</strong>;
      case 'text':
        return <Fragment key={key}>{piece.text}</Fragment>;
    }
  });
}
