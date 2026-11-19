import { readFileSync } from 'node:fs';
import { fileURLToPath } from 'node:url';

import { describe, expect, it } from 'vitest';

import { desktopSavedExtensions, desktopSavesFile } from './saved-file-types';

function nativeExtensions(): readonly string[] {
  const source = readFileSync(
    fileURLToPath(new URL('../../../../../desktop/src-tauri/src/saved_files.rs', import.meta.url)),
    'utf8',
  );
  const list = /const ALLOWED_EXTENSIONS: &\[&str\] = &\[([^\]]*)\];/u.exec(source)?.[1];
  if (list === undefined) throw new Error('原生层的 ALLOWED_EXTENSIONS 找不到了');
  return Array.from(list.matchAll(/"([^"]+)"/gu), (match) => match[1] ?? '');
}

describe('桌面应用肯存的文件', () => {
  it('和原生层的名单一样', () => {
    expect([...desktopSavedExtensions].sort()).toEqual([...nativeExtensions()].sort());
  });

  it('文本、位图和 PDF 存', () => {
    for (const name of [
      'alpha98-attachment.txt',
      '发布验收 记录.MD',
      'report.final.pdf',
      'photo.JPEG',
      'build.log',
      ' notes.txt ',
      '.png',
    ]) {
      expect(desktopSavesFile(name), name).toBe(true);
    }
  });

  it('能运行的、点开是浏览器的、压缩包和 Office 文档不存', () => {
    for (const name of [
      'setup.exe',
      'run.bat',
      'page.html',
      'drawing.svg',
      'tool.py',
      'bundle.zip',
      'report.docx',
      'notes.txt.exe',
      'notes.txt.',
      'notes.txt:evil.exe',
      'README',
      '',
    ]) {
      expect(desktopSavesFile(name), name).toBe(false);
    }
  });

  it('只把 ASCII 字母当成不分大小写', () => {
    // 开尔文符号小写以后是 k：原生层不认，这里也不认。
    expect(desktopSavesFile('notes.tKxt')).toBe(false);
    expect(desktopSavesFile('notes.TXT')).toBe(true);
  });
});
