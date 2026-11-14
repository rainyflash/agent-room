/**
 * 窗口在不在前台：标签页看得见，窗口也拿着焦点。
 * 开在别的标签页后面、最小化、或者别的程序在前面时都不算：这时来的消息人没看到。
 */
export function windowInForeground(target: Document = document): boolean {
  return target.visibilityState === 'visible' && target.hasFocus();
}

/** 窗口到了前台、离开前台时叫一声；返回的函数取消。 */
export function onWindowForegroundChange(
  listener: () => void,
  target: Window = window,
): () => void {
  target.document.addEventListener('visibilitychange', listener);
  target.addEventListener('focus', listener);
  target.addEventListener('blur', listener);
  return () => {
    target.document.removeEventListener('visibilitychange', listener);
    target.removeEventListener('focus', listener);
    target.removeEventListener('blur', listener);
  };
}
