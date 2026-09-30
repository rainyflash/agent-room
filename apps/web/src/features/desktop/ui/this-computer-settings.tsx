import { Button, Details } from '@agent-room/ui-system';
import { Link } from '@tanstack/react-router';
import { FolderOpen } from 'lucide-react';
import { useTranslation } from 'react-i18next';

import { ApplicationUpdates } from '@/features/updates/ui/application-updates';
import { useDesktopRuntimeController } from './desktop-runtime-provider';
import { McpConfiguration } from './mcp-configuration';
import './this-computer.css';

/**
 * “设置 → 这台电脑”：登录后自动打开、应用更新（只有这一处能装）、日志文件夹、MCP 配置。
 * 这台电脑的连接状态和接进来的 Agent 在“我的 Agent”里。
 */
export function ThisComputerSettings() {
  const { t } = useTranslation();
  const controller = useDesktopRuntimeController();
  const snapshot = controller.snapshot;
  if (!controller.available || snapshot === null) return null;
  return (
    <div className="settings-rows this-computer-settings">
      <p className="settings-intro">
        {t('settings.thisComputer.intro')}{' '}
        <Link hash="this-computer" search={{}} to="/workspace">
          {t('settings.thisComputer.agents')}
        </Link>
      </p>
      <div className="settings-row">
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
      <div className="settings-row settings-row--block">
        <ApplicationUpdates />
      </div>
      <div className="settings-row">
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
    </div>
  );
}
