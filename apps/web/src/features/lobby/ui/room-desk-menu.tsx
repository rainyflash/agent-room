import { useLayoutEffect, useRef, type ReactNode, type RefObject } from 'react';

/**
 * 顶栏上点开的一小块：左栏收起来时的“主房间和私聊”，手机上的“更多”
 * （`specs/redesign/room-page.md` 的“布局”“手机宽度”）。
 *
 * 原生 `<dialog>`，模态：Esc、点外面都关，关了焦点回到打开它的按钮。宽一些的窗口里它贴在打开它的
 * 按钮下面，手机上是从下面出来的一层，样式在 `room-desk.css`。
 * 里面是普通的按钮，不是 `role="menu"`：Tab 走得到每一项，不用另学方向键。
 *
 * 打开它的按钮要明着给（`anchor`），不看此刻焦点在谁身上：Safari 里用鼠标点按钮不给它焦点，
 * 那样就量不到按钮在哪儿，关了焦点也回不去。
 */
export function RoomDeskMenu({
  anchor,
  children,
  label,
  onClose,
}: {
  readonly anchor: RefObject<HTMLElement | null>;
  readonly children: ReactNode;
  readonly label: string;
  readonly onClose: () => void;
}) {
  const dialog = useRef<HTMLDialogElement>(null);
  useLayoutEffect(() => {
    const element = dialog.current;
    if (element === null) return;
    const trigger = anchor.current;
    // 贴在打开它的按钮下面、左边对齐（手机上的样式不看这两个数）。
    if (trigger !== null) {
      const box = trigger.getBoundingClientRect();
      element.style.setProperty('--room-desk-menu-top', `${String(Math.round(box.bottom + 6))}px`);
      element.style.setProperty('--room-desk-menu-left', `${String(Math.round(box.left))}px`);
    }
    // 没有 `showModal` 的环境（单元测试）退化成 `open` 属性。
    if (typeof element.showModal === 'function') element.showModal();
    else element.setAttribute('open', '');
    return () => {
      if (typeof element.close === 'function') element.close();
      else element.removeAttribute('open');
      if (trigger?.isConnected === true) trigger.focus();
    };
  }, [anchor]);
  return (
    <dialog
      aria-label={label}
      className="room-desk-menu"
      onCancel={(event) => {
        event.preventDefault();
        onClose();
      }}
      // 里面那一层铺满了整块：点到 `<dialog>` 自己，就是点在外面那层底上。
      onClick={(event) => {
        if (event.target === event.currentTarget) onClose();
      }}
      ref={dialog}
    >
      <div className="room-desk-menu__body">{children}</div>
    </dialog>
  );
}
