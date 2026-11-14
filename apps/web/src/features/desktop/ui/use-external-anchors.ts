import { useEffect } from 'react';

/**
 * 桌面端的窗口打不开新窗口：页面里带 `target="_blank"` 的链接（发行说明、源代码、分享到别处）
 * 点了什么也不发生。这里接住这些点击，交给原生层在系统浏览器里打开。
 * 只管 `http`、`https`；页面自己已经处理掉的点击不管。
 */
export function useExternalAnchors(open: ((url: string) => Promise<boolean>) | undefined): void {
  useEffect(() => {
    if (open === undefined) return;
    const onClick = (event: MouseEvent): void => {
      if (event.defaultPrevented || event.button !== 0) return;
      const anchor = event.target instanceof Element ? event.target.closest('a') : null;
      if (anchor?.target !== '_blank') return;
      if (anchor.protocol !== 'http:' && anchor.protocol !== 'https:') return;
      event.preventDefault();
      void open(anchor.href);
    };
    document.addEventListener('click', onClick);
    return () => {
      document.removeEventListener('click', onClick);
    };
  }, [open]);
}
