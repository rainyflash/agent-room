import { Download } from 'lucide-react';
import { useTranslation } from 'react-i18next';

import { useOptionalAppServices } from '@/app/app-services';
import { useOptionalDesktopRuntimeController } from '@/features/desktop/ui/desktop-runtime-provider';
import { GitHubMark } from '@/shared/ui/github-mark';
import {
  REPOSITORY_URL,
  usePublishedDownload,
  type DownloadConfiguration,
} from './use-published-download';

/**
 * 网页顶栏右边：GitHub 上的项目（只放标志），和按访客的系统给的安装包。桌面端里不显示，更新在
 * “设置”里；访客的系统没有安装包（手机、Linux）时只给 GitHub。下载按钮上写“下载应用”，
 * 是哪个系统的写在提示里；窄屏上只留图标，名字照样读得出来。
 */
export function WebHeaderLinks() {
  const desktop = useOptionalDesktopRuntimeController();
  const services = useOptionalAppServices();
  if (desktop?.available === true || services === null) return null;
  return <WebHeaderLinkList config={services.config} />;
}

function WebHeaderLinkList({ config }: { readonly config: DownloadConfiguration }) {
  const { t } = useTranslation();
  const download = usePublishedDownload(config);
  const source = t('application.source');
  return (
    <>
      <a
        aria-label={source}
        className="ar-button ar-button--ghost ar-button--default app-navigation__external app-navigation__external--github"
        href={REPOSITORY_URL}
        rel="noreferrer"
        target="_blank"
        title={source}
      >
        <span className="ar-button__icon">
          <GitHubMark />
        </span>
      </a>
      {download.url === null ? null : (
        <a
          className="ar-button ar-button--primary ar-button--default app-navigation__external app-navigation__external--download"
          href={download.url}
          rel="noreferrer"
          target="_blank"
          title={t(
            download.platform === 'macos' ? 'landing.download.macos' : 'landing.download.windows',
          )}
        >
          <span className="ar-button__icon">
            <Download aria-hidden="true" />
          </span>
          <span>{t('navigation.download')}</span>
        </a>
      )}
    </>
  );
}
