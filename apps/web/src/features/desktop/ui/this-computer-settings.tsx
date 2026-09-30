import { Button, Details } from '@agent-room/ui-system';
import { Link } from '@tanstack/react-router';
import { FolderOpen } from 'lucide-react';
import { useTranslation } from 'react-i18next';

import { useDesktopRuntimeController } from './desktop-runtime-provider';
import { McpConfiguration } from './mcp-configuration';

/**
 * 这台电脑的设置：登录后自动打开、应用更新、日志文件夹、MCP 配置。界面翻新第 4 步会把它们
 * 搬进“设置”，在那之前放在“这台电脑”一节里，免得右下角面板拆掉后找不到。
 */
export function ThisComputerSettings() {
  const { t } = useTranslation();
  const controller = useDesktopRuntimeController();
  const snapshot = controller.snapshot;
  if (snapshot === null) return null;
  const version = snapshot.currentVersion ?? controller.update?.currentVersion ?? null;
  const update = controller.update?.available === true ? controller.update : null;
  return (
    <section aria-labelledby="this-computer-settings-title" className="this-computer__settings">
      <h3 id="this-computer-settings-title">{t('thisComputer.settings.title')}</h3>
      <div className="this-computer__row">
        <div>
          <strong>{t('desktop.autostart.title')}</strong>
          <p>
            {t('desktop.autostart.description', {
              platform: t(`desktop.platform.${snapshot.platform}`),
            })}
          </p>
        </div>
        <button
          aria-pressed={snapshot.autostartEnabled}
          className="this-computer__switch"
          disabled={controller.busy !== null}
          onClick={() => void controller.setAutostart(!snapshot.autostartEnabled)}
          type="button"
        >
          <span aria-hidden="true" />
          {snapshot.autostartEnabled ? t('desktop.autostart.on') : t('desktop.autostart.off')}
        </button>
      </div>
      <div className="this-computer__row">
        <div>
          <strong>{t('thisComputer.updates.title')}</strong>
          {version === null ? null : <p>{t('thisComputer.updates.version', { version })}</p>}
        </div>
        <Link
          className={`ar-button ar-button--compact ${update === null ? 'ar-button--ghost' : 'ar-button--primary'}`}
          to="/about"
        >
          {update === null
            ? t('thisComputer.updates.open')
            : t('desktop.update.badge', { version: update.targetVersion })}
        </Link>
      </div>
      <div className="this-computer__row">
        <div>
          <strong>{t('thisComputer.logs.title')}</strong>
          <p>{t('desktop.logs.hint')}</p>
        </div>
        <Button
          icon={<FolderOpen aria-hidden="true" />}
          onClick={() => void controller.openLogs()}
          size="compact"
          tone="ghost"
        >
          {t('desktop.logs.open')}
        </Button>
      </div>
      <Details summary={t('agentInvite.mode.mcp')}>
        <McpConfiguration configuration={snapshot.manualHostConfiguration} />
      </Details>
    </section>
  );
}
