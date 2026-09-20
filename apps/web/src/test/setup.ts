import '@testing-library/jest-dom/vitest';

// jsdom 默认把宿主平台写进 User-Agent，而页面按访客系统给下载入口：不固定下来，
// 同一份测试在 Windows 开发机和 Linux runner 上结果不同。要验别的系统，
// 在用例里 vi.stubGlobal('navigator', …) 换掉即可。
Object.defineProperty(window.navigator, 'userAgent', {
  configurable: true,
  value:
    'Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 (KHTML, like Gecko) jsdom/26.1.0',
});
