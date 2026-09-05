import { OverlayContainerProvider } from '@/shared/ui/overlay-container';
import { X } from 'lucide-react';
import { useLayoutEffect, useRef, useState, type ReactNode } from 'react';
import { useTranslation } from 'react-i18next';

export function WorkspaceDrawer({
  children,
  label,
  onClose,
  variant,
}: {
  readonly children: ReactNode;
  readonly label: string;
  readonly onClose: () => void;
  readonly variant: 'navigation' | 'members';
}) {
  const { t } = useTranslation();
  const dialog = useRef<HTMLDialogElement>(null);
  const [overlayContainer, setOverlayContainer] = useState<HTMLDivElement | null>(null);
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
      className={`workspace-drawer workspace-drawer--${variant}`}
      ref={dialog}
      onCancel={(event) => {
        if (dialog.current?.querySelector('[role="dialog"][aria-modal="true"]') != null)
          event.preventDefault();
        else onClose();
      }}
      onClose={onClose}
    >
      <button
        className="workspace-drawer__close"
        type="button"
        aria-label={t('roomWorkspace.closePanel')}
        onClick={onClose}
      >
        <X aria-hidden="true" />
      </button>
      <OverlayContainerProvider container={overlayContainer}>{children}</OverlayContainerProvider>
      <div className="workspace-drawer__overlays" ref={setOverlayContainer} />
    </dialog>
  );
}
