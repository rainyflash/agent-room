import type { TFunction } from 'i18next';
import { Banner, CopyBlock, Details } from '@agent-room/ui-system';
import { Download } from 'lucide-react';
import { useTranslation } from 'react-i18next';

import { useOnboardingStep } from '@/features/telemetry/ui/use-onboarding-step';

import { cliInvocation, quoteCliArgument } from '../domain/cli-invitation';
import type { DesktopRuntimeSnapshot } from '../domain/desktop-runtime';
import type { InviteRoom } from './agent-invite-dialog';
import { useDesktopRuntimeController } from './desktop-runtime-provider';
import { LocalConnectionNotice } from './local-connection-notice';
import { McpConfiguration } from './mcp-configuration';

type LocalMethod = 'mcp' | 'cli';

/**
 * MCP 与命令行：两种都按房间名接入，同一个任务再接入会回到同一个人物，所以只要一段话。
 * MCP 第一次用要把配置加进工具，放在折叠里；网页端看不到 Agent 那台电脑，只说要装 Agent Room。
 */
export function LocalAgentInvite({
  method,
  room,
  agentName,
  downloadUrl,
  onCopied,
}: {
  readonly method: LocalMethod;
  readonly room: InviteRoom;
  /** 人替它起的名字；空的时候话里请 Agent 自己起。 */
  readonly agentName: string;
  readonly downloadUrl: string | null;
  readonly onCopied: () => void;
}) {
  const { t } = useTranslation();
  const controller = useDesktopRuntimeController();
  const desktop = controller.available;
  const snapshot = controller.snapshot;
  const cliMissing = desktop && method === 'cli' && snapshot?.cliConfiguration === null;
  // 这台电脑还没授权时，Agent 照着做也进不来：先把授权做完。
  const needsAuthorization =
    desktop && snapshot?.bridge.lifecycle.phase === 'authorization_required';
  const mcpNaming =
    agentName === ''
      ? t('agentInvite.naming.mcp.choose')
      : t('agentInvite.naming.mcp.given', { name: agentName });
  const message =
    method === 'mcp'
      ? room === null
        ? t('agentInvite.mcp.messageLobby', { naming: mcpNaming })
        : t('agentInvite.mcp.message', { naming: mcpNaming, room: room.roomName })
      : cliMessage(t, room, snapshot, desktop, agentName);

  return (
    <>
      {desktop ? (
        <LocalConnectionNotice />
      ) : (
        <WebNotice downloadUrl={downloadUrl} method={method} />
      )}
      {cliMissing ? (
        <Banner tone="warning">{t('agentInvite.cli.missing')}</Banner>
      ) : (
        <CopyBlock
          copiedLabel={t('agentInvite.message.copied')}
          copyLabel={t('agentInvite.message.copy')}
          disabled={needsAuthorization}
          failedLabel={t('agentInvite.message.failed')}
          onCopied={onCopied}
          text={message}
          textLabel={t('agentInvite.message.label')}
        />
      )}
      {method === 'mcp' && desktop && snapshot !== null ? (
        <>
          <p className="agent-invite__note">{t('agentInvite.mcp.say')}</p>
          <Details summary={t('agentInvite.mcp.setup')}>
            <McpConfiguration configuration={snapshot.manualHostConfiguration} />
          </Details>
        </>
      ) : null}
    </>
  );
}

/** 桌面端用实际安装路径；网页端不知道 Agent 那台电脑装在哪，多给一句去哪找。 */
function cliMessage(
  t: TFunction,
  room: InviteRoom,
  snapshot: DesktopRuntimeSnapshot | null,
  desktop: boolean,
  agentName: string,
): string {
  const platform = snapshot?.platform ?? 'unknown';
  const invocation = cliInvocation(snapshot?.cliConfiguration, platform);
  const join =
    room === null
      ? `${invocation} join`
      : `${invocation} join --room ${quoteCliArgument(room.roomName, platform)}`;
  const command =
    agentName === '' ? join : `${join} --name ${quoteCliArgument(agentName, platform)}`;
  const text = t(room === null ? 'agentInvite.cli.messageLobby' : 'agentInvite.cli.message', {
    command,
    guide: `${invocation} guide`,
    naming: agentName === '' ? t('agentInvite.naming.cli.choose') : '',
    room: room?.roomName ?? '',
  });
  return desktop ? text : `${text}\n\n${t('agentInvite.cli.locate')}`;
}

/** 网页端：MCP 和命令行都要 Agent 那台电脑装着 Agent Room。 */
function WebNotice({
  method,
  downloadUrl,
}: {
  readonly method: LocalMethod;
  readonly downloadUrl: string | null;
}) {
  const { t } = useTranslation();
  const reportStep = useOnboardingStep();
  return (
    <Banner
      action={
        downloadUrl === null ? undefined : (
          <a
            className="ar-button ar-button--ghost ar-button--compact"
            href={downloadUrl}
            onClick={() => {
              reportStep({ step: 'desktop_download' });
            }}
          >
            <span className="ar-button__icon">
              <Download aria-hidden="true" />
            </span>
            <span>{t('agentInvite.web.download')}</span>
          </a>
        )
      }
      role={null}
      tone="info"
    >
      {t(method === 'mcp' ? 'agentInvite.web.mcp' : 'agentInvite.web.cli')}
    </Banner>
  );
}
