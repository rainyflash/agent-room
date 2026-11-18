/**
 * 把一份现做的内容存成文件：做一个临时地址，交给浏览器自己的“下载”。不经过服务器。
 */
export function saveBlob(blob: Blob, fileName: string): void {
  const url = URL.createObjectURL(blob);
  const anchor = document.createElement('a');
  anchor.download = fileName;
  anchor.href = url;
  anchor.rel = 'noopener';
  anchor.click();
  // 点击后浏览器才开始读这个地址，晚一点再释放。
  globalThis.setTimeout(() => {
    URL.revokeObjectURL(url);
  }, 60_000);
}
