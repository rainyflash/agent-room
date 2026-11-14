import { X } from 'lucide-react';
import { useLayoutEffect, useRef, type ReactNode } from 'react';
import { useTranslation } from 'react-i18next';

/**
 * 房间页上盖着的一层：窄的时候的 Agent 和人。平时从右边出来；`sheet` 的在手机上是从下面出来的
 * 一整页（对话版面用，样式在 `lobby-workspace.css`）。
 */
export function WorkspaceDrawer({
  children,
  label,
  onClose,
  sheet = false,
  variant,
}: {
  readonly children: ReactNode;
  readonly label: string;
  readonly onClose: () => void;
  readonly sheet?: boolean;
  readonly variant: 'members';
}) {
  const { t } = useTranslation();
  const dialog = useRef<HTMLDialogElement>(null);
  const pressedOutside = useRef(false);
  useLayoutEffect(() => {
    const element = dialog.current;
    if (element === null) return;
    const trigger = document.activeElement;
    element.showModal();
    return () => {
      element.close();
      if (trigger instanceof HTMLElement && trigger.isConnected) trigger.focus();
    };
  }, []);
  return (
    <dialog
      aria-label={label}
      className={`workspace-drawer workspace-drawer--${variant}${sheet ? ' workspace-drawer--sheet' : ''}`}
      ref={dialog}
      // 点在它外面那层底上就关：按下、松开都在外面才算（在里面按下、拖到外面松开的不算，
      // 比如拖着选搜索框里的字）。
      onPointerDown={(event) => {
        pressedOutside.current = outsideOf(event.currentTarget, event);
      }}
      onClick={(event) => {
        const pressed = pressedOutside.current;
        pressedOutside.current = false;
        if (!pressed || !outsideOf(event.currentTarget, event)) return;
        // 里面还开着一层模态的对话框时，先关的该是它。
        if (event.currentTarget.querySelector('[role="dialog"][aria-modal="true"]') !== null)
          return;
        onClose();
      }}
      onCancel={(event) => {
        if (dialog.current?.querySelector('[role="dialog"][aria-modal="true"]') != null)
          event.preventDefault();
        else onClose();
      }}
      onClose={onClose}
    >
      <button
        className="workspace-drawer__close ar-icon-button"
        type="button"
        aria-label={t('roomWorkspace.closePanel')}
        onClick={onClose}
      >
        <X aria-hidden="true" />
      </button>
      {children}
    </dialog>
  );
}

/**
 * 这一下是不是落在这一层外面（它后面那层暗的底上）。`<dialog>` 自己有留白，点到留白时事件的
 * 目标也是它，所以按位置认。
 */
function outsideOf(
  element: HTMLElement,
  point: { readonly clientX: number; readonly clientY: number },
): boolean {
  const box = element.getBoundingClientRect();
  return (
    point.clientX < box.left ||
    point.clientX > box.right ||
    point.clientY < box.top ||
    point.clientY > box.bottom
  );
}
