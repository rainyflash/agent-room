import { useEffect, useMemo, useRef } from 'react';
import { TypingNotice } from '../application/typing-notice';
import type { TypingNotifier } from '../domain/typing';

/**
 * 输入框内容一变就告诉房间“正在输入”；不能输入或正在发送时说停了。
 * 进房间时恢复出来的草稿不算打字。
 */
export function useTypingNotice(
  notifier: TypingNotifier | undefined,
  roomId: string,
  text: string,
  active: boolean,
): void {
  const notice = useMemo(
    () => (notifier === undefined ? null : new TypingNotice(notifier, roomId)),
    [notifier, roomId],
  );
  useEffect(
    () => () => {
      notice?.stop();
    },
    [notice],
  );
  const seen = useRef(text);
  useEffect(() => {
    if (notice === null) return;
    if (!active) {
      seen.current = text;
      notice.stop();
      return;
    }
    if (seen.current === text) return;
    seen.current = text;
    notice.changed(text);
  }, [notice, text, active]);
}
