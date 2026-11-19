/**
 * 桌面应用肯替页面存进“下载”文件夹的几种文件：点开以后不会运行的文本、位图和 PDF。
 *
 * 和原生层 `saved_files.rs` 的 `ALLOWED_EXTENSIONS` 是同一份，测试卡着两边一样。原生层才是作数的那一份：
 * 这里只用来先告诉人“这种存不了”，免得点了才报没存成。要多存一种先改原生层，那里写着为什么别的不收。
 */
export const desktopSavedExtensions = [
  'png',
  'json',
  'txt',
  'md',
  'log',
  'csv',
  'tsv',
  'yaml',
  'yml',
  'toml',
  'diff',
  'patch',
  'jpg',
  'jpeg',
  'gif',
  'webp',
  'avif',
  'pdf',
] as const;

/**
 * 桌面应用存不存叫这个名字的文件。和原生层一样：只看最后一个点后面那一段，只把 ASCII 字母当成不分大小写。
 */
export function desktopSavesFile(fileName: string): boolean {
  const trimmed = fileName.trim();
  const dot = trimmed.lastIndexOf('.');
  if (dot < 0) return false;
  const extension = trimmed.slice(dot + 1).replace(/[A-Z]/gu, (letter) => letter.toLowerCase());
  return (desktopSavedExtensions as readonly string[]).includes(extension);
}
