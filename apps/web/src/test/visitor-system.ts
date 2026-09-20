/**
 * 页面按访客的系统给下载入口，而 jsdom 默认把运行主机的平台写进 User-Agent：
 * 用例必须自己点名系统，否则同一份断言在 Windows 开发机和 Linux runner 上结果不同。
 */
export function setVisitorSystem(userAgent: string): void {
  Object.defineProperty(window.navigator, 'userAgent', { configurable: true, value: userAgent });
}

export const WINDOWS_VISITOR = 'Mozilla/5.0 (Windows NT 10.0; Win64; x64)';
export const MACOS_VISITOR = 'Mozilla/5.0 (Macintosh; Intel Mac OS X 10_15_7)';
export const LINUX_VISITOR = 'Mozilla/5.0 (X11; Linux x86_64)';
