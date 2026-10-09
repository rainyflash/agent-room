import { Download, ExternalLink, Laptop } from 'lucide-react';
import { useTranslation } from 'react-i18next';

import { RELEASES_URL, type PublishedDownload } from './use-published-download';
import './desktop-app-card.css';

/**
 * 网页端“我的 Agent”里的“桌面应用”一节，放在桌面端“这台电脑”那一节的位置：登录以后也找得到下载。
 * 按访客的系统给安装包；没有这个系统的安装包时按钮变灰，GitHub 上的所有版本总是给。
 */
export function DesktopAppCard({ download }: { readonly download: PublishedDownload }) {
  const { t } = useTranslation();
  return (
    <section aria-labelledby="desktop-app-title" className="desktop-app-card">
      <span aria-hidden="true" className="desktop-app-card__icon">
        <Laptop />
      </span>
      <div className="desktop-app-card__text">
        <h2 id="desktop-app-title">{t('desktopApp.title')}</h2>
        <p>{t('desktopApp.description')}</p>
      </div>
      <div className="desktop-app-card__actions">
        {download.url === null ? (
          <button className="ar-button ar-button--default ar-button--ghost" disabled type="button">
            <Download aria-hidden="true" />
            {t('landing.downloadPending')}
          </button>
        ) : (
          <a
            className="ar-button ar-button--default ar-button--primary"
            href={download.url}
            rel="noreferrer"
            target="_blank"
          >
            <Download aria-hidden="true" />
            {t(
              download.platform === 'macos' ? 'landing.download.macos' : 'landing.download.windows',
            )}
          </a>
        )}
        <a
          className="desktop-app-card__versions"
          href={RELEASES_URL}
          rel="noreferrer"
          target="_blank"
        >
          {t('desktopApp.allVersions')}
          <ExternalLink aria-hidden="true" />
        </a>
      </div>
    </section>
  );
}
