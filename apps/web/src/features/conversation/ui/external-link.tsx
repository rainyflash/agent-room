import { Banner, Button, CopyBlock, Dialog } from '@agent-room/ui-system';
import { createContext, useContext, useState, type PropsWithChildren } from 'react';
import { useTranslation } from 'react-i18next';
import { useOptionalDesktopRuntimeController } from '@/features/desktop/ui/desktop-runtime-provider';
import './external-link.css';

type LinkRequest = (href: string) => void;

const ExternalLinkContext = createContext<LinkRequest | null>(null);

/** 正文里的地址点了交给谁。没人接的地方（围观页、头顶的气泡）回 `null`，地址就只是字。 */
export function useExternalLinkRequest(): LinkRequest | null {
  return useContext(ExternalLinkContext);
}

/**
 * 正文里的地址不直接打开：点了先在这里问一句，写出域名和完整的地址，确认了才打开。
 * 房间里谁都能发地址，Agent 发的尤其多；字面上看着像认识的网站，不等于就是它。
 */
export function ExternalLinkGate({ children }: PropsWithChildren) {
  const [pending, setPending] = useState<string | null>(null);
  return (
    <ExternalLinkContext.Provider value={setPending}>
      {children}
      {pending === null ? null : (
        <ExternalLinkDialog
          href={pending}
          onClose={() => {
            setPending(null);
          }}
        />
      )}
    </ExternalLinkContext.Provider>
  );
}

function ExternalLinkDialog({
  href,
  onClose,
}: {
  readonly href: string;
  readonly onClose: () => void;
}) {
  const { t } = useTranslation();
  const desktop = useOptionalDesktopRuntimeController();
  // 桌面端的窗口自己打不开新窗口，交给原生层在系统浏览器里打开。
  const openNatively = desktop?.available === true ? desktop.openLink : undefined;
  const [state, setState] = useState<'asking' | 'opening' | 'failed'>('asking');

  const open = async (): Promise<void> => {
    if (openNatively === undefined) {
      // 不带来源：新开的页面拿不到这个窗口，也不知道是从哪儿点过去的。
      window.open(href, '_blank', 'noopener,noreferrer');
      onClose();
      return;
    }
    setState('opening');
    if (await openNatively(href)) onClose();
    else setState('failed');
  };

  return (
    <Dialog
      className="external-link"
      closeLabel={t('conversation.link.close')}
      description={t(
        openNatively === undefined
          ? 'conversation.link.opensInTab'
          : 'conversation.link.opensInBrowser',
      )}
      footer={
        <>
          <Button onClick={onClose} tone="quiet">
            {t('conversation.link.cancel')}
          </Button>
          <CopyBlock
            copiedLabel={t('conversation.link.copied')}
            copyLabel={t('conversation.link.copy')}
            failedLabel={t('conversation.link.copyFailed')}
            hideText
            size="default"
            text={href}
            tone="ghost"
          />
          <Button disabled={state === 'opening'} onClick={() => void open()}>
            {t('conversation.link.open')}
          </Button>
        </>
      }
      onClose={onClose}
      title={t('conversation.link.title')}
    >
      <dl className="external-link__facts">
        <div>
          <dt>{t('conversation.link.site')}</dt>
          <dd className="external-link__site">{new URL(href).host}</dd>
        </div>
        <div>
          <dt>{t('conversation.link.address')}</dt>
          <dd className="external-link__address">{href}</dd>
        </div>
      </dl>
      <p className="external-link__note">{t('conversation.link.note')}</p>
      {state === 'failed' ? <Banner tone="danger">{t('conversation.link.failed')}</Banner> : null}
    </Dialog>
  );
}
