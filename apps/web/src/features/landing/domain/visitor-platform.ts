/** 首页按访客的系统给出合适的入门路径：Windows 有安装器，其余系统先用浏览器。 */
export type VisitorPlatform = 'windows' | 'macos' | 'linux' | 'other';

/** 只看系统，不猜架构；识别不出时按 other 处理，给出对所有系统都成立的说明。 */
export function detectVisitorPlatform(userAgent: string): VisitorPlatform {
  const value = userAgent.toLowerCase();
  // iPadOS 的桌面模式会自称 Macintosh，所以先看触摸设备特征。
  if (/iphone|ipad|ipod|android/u.test(value)) {
    return 'other';
  }
  if (/windows|win32|win64/u.test(value)) {
    return 'windows';
  }
  if (/macintosh|mac os x/u.test(value)) {
    return 'macos';
  }
  if (/linux|x11|cros/u.test(value)) {
    return 'linux';
  }
  return 'other';
}
