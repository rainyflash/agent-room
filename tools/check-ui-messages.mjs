import { readFile, readdir } from 'node:fs/promises';
import path from 'node:path';
import process from 'node:process';

import ts from 'typescript';

const sourceRoot = path.resolve('apps/web/src');
const visibleAttributes = new Set(['alt', 'aria-label', 'placeholder', 'title']);
const ignoredFileSuffixes = ['.spec.tsx', '.test.tsx'];

const files = await collectTsxFiles(sourceRoot);
const violations = [];

for (const file of files) {
  const sourceText = await readFile(file, 'utf8');
  const source = ts.createSourceFile(
    file,
    sourceText,
    ts.ScriptTarget.Latest,
    true,
    ts.ScriptKind.TSX,
  );
  inspect(source, source);
}

if (violations.length > 0) {
  process.stderr.write('发现未进入类型化消息目录的可见 UI 文案：\n');
  for (const violation of violations) {
    process.stderr.write(
      `- ${path.relative(process.cwd(), violation.file)}:${violation.line} ${JSON.stringify(violation.text)}\n`,
    );
  }
  process.exitCode = 1;
} else {
  process.stdout.write(`UI 文案检查通过（${files.length} 个 TSX 文件）。\n`);
}

async function collectTsxFiles(directory) {
  const entries = await readdir(directory, { withFileTypes: true });
  const collected = [];
  for (const entry of entries) {
    const absolute = path.join(directory, entry.name);
    if (entry.isDirectory()) {
      if (entry.name !== 'test') {
        collected.push(...(await collectTsxFiles(absolute)));
      }
      continue;
    }
    if (
      entry.name.endsWith('.tsx') &&
      !ignoredFileSuffixes.some((suffix) => entry.name.endsWith(suffix))
    ) {
      collected.push(absolute);
    }
  }
  return collected;
}

function inspect(node, source) {
  if (ts.isJsxText(node)) {
    recordIfVisible(node.getText(source), node, source);
  }

  if (ts.isJsxAttribute(node) && visibleAttributes.has(node.name.getText(source))) {
    const initializer = node.initializer;
    if (initializer !== undefined && ts.isStringLiteral(initializer)) {
      recordIfVisible(initializer.text, initializer, source);
    }
  }

  if (
    ts.isJsxExpression(node) &&
    node.expression !== undefined &&
    (ts.isStringLiteral(node.expression) || ts.isNoSubstitutionTemplateLiteral(node.expression))
  ) {
    recordIfVisible(node.expression.text, node.expression, source);
  }

  ts.forEachChild(node, (child) => inspect(child, source));
}

function recordIfVisible(rawText, node, source) {
  const text = rawText.replace(/\s+/g, ' ').trim();
  if (!/[A-Za-z\u3400-\u9fff]/u.test(text)) {
    return;
  }
  const position = source.getLineAndCharacterOfPosition(node.getStart(source));
  violations.push({ file: source.fileName, line: position.line + 1, text });
}
