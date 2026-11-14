import { useEffect } from 'react';

/**
 * 有没看到的消息时，在窗口标题前面写上条数，比如“(3) Agent Room”：人在别的标签页、
 * 别的程序里时，只有标题看得到。没有了、离开这一页时改回去。
 */
export function useUnreadTitle(count: number): void {
  useEffect(() => {
    if (count <= 0) return;
    const base = document.title;
    document.title = `(${count > 99 ? '99+' : String(count)}) ${base}`;
    return () => {
      document.title = base;
    };
  }, [count]);
}
