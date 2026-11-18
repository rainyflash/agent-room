import type { TFunction } from 'i18next';
import { Banner, Button, CopyBlock, Details } from '@agent-room/ui-system';
import { Download } from 'lucide-react';
import { useTranslation } from 'react-i18next';

import { useOnboardingStep } from '@/features/telemetry/ui/use-onboarding-step';

import { cliInvocation, quoteCliArgument } from '../domain/cli-invitation';
import type { DesktopRuntimeSnapshot } from '../domain/desktop-runtime';
import type { InviteRoom } from './agent-invite-dialog';
import { useDesktopRuntimeController } from './desktop-runtime-provider';
import { LocalConnectionNotice } from './local-connection-notice';
import { McpConfiguration } from './mcp-configuration';
import type { AccountComputer } from './use-account-computer';

type LocalMethod = 'mcp' | 'cli';

/** 网页端才用得上的几样：这个账号有没有装着 Agent Room 的电脑、点没点过下载、不想装时的退路。 */
export type WebInviteSetup = {
  readonly computer: AccountComputer;
  readonly started: boolean;
  readonly onStarted: () => void;
  readonly onUseNetwork: () => void;
};

/**
 * MCP 与命令行：两种都按房间名接入，同一个任务再接入会回到同一个人物，所以只要一段话。
 * MCP 第一次用要把配置加进工具，放在折叠里；网页端看不到 Agent 那台电脑，账号还没有装着
 * Agent Room 的电脑时，头一步是请人下载。
 */
export function LocalAgentInvite({
  method,
  room,
  agentName,
  background = false,
  downloadUrl,
  onCopied,
  web,
}: {
  readonly method: LocalMethod;
  readonly room: InviteRoom;
  /** 人替它起的名字；空的时候话里请 Agent 自己起。 */
  readonly agentName: string;
  /** 勾着“你不在时也让它回话”：话里多一句，请它登记后台回复。 */
  readonly background?: boolean;
  readonly downloadUrl: string | null;
  readonly onCopied: () => void;
  readonly web: WebInviteSetup;
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
  const invitation =
    method === 'mcp'
      ? room === null
        ? t('agentInvite.mcp.messageLobby', { naming: mcpNaming })
        : t('agentInvite.mcp.message', { naming: mcpNaming, room: room.roomName })
      : cliMessage(t, room, snapshot, desktop, agentName);
  const message = background
    ? `${invitation}\n${t(`agentInvite.background.${method}`)}`
    : invitation;
  // 网页端还要先装桌面应用时，明黄的按钮是下载；点过下载、或者账号已经有电脑，才轮到复制。
  const installFirst = !desktop && downloadUrl !== null && web.computer !== 'some' && !web.started;

  return (
    <>
      {desktop ? (
        <LocalConnectionNotice />
      ) : (
        <WebSetup downloadUrl={downloadUrl} method={method} web={web} />
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
          tone={installFirst ? 'ghost' : 'primary'}
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

/**
 * 网页端：MCP 和命令行都要 Agent 那台电脑装着 Agent Room。
 * - 访客这个系统没有安装包（手机、Linux）：只说要装，没有下载。
 * - 账号已经有装着它的电脑：一句话带过，下载留着给另一台电脑。
 * - 还没有：这是头一步，下载是这里唯一的明黄按钮；不想装的旁边就能换成网络。
 */
function WebSetup({
  method,
  downloadUrl,
  web,
}: {
  readonly method: LocalMethod;
  readonly downloadUrl: string | null;
  readonly web: WebInviteSetup;
}) {
  const { t } = useTranslation();
  if (downloadUrl === null) {
    return (
      <Banner role={null} tone="info">
        {t(method === 'mcp' ? 'agentInvite.web.mcp' : 'agentInvite.web.cli')}
      </Banner>
    );
  }
  // 还在问账号有没有电脑的这一小会儿先空着，免得“先装”闪一下又换掉。
  if (web.computer === 'unknown') return null;
  if (web.computer === 'some') {
    return (
      <Banner
        action={<DownloadLink compact downloadUrl={downloadUrl} tone="ghost" />}
        role={null}
        tone="info"
      >
        {t(method === 'mcp' ? 'agentInvite.web.installedMcp' : 'agentInvite.web.installedCli')}
      </Banner>
    );
  }
  return (
    <section className="agent-invite__setup">
      <h3>{t('agentInvite.web.setup.title')}</h3>
      <p aria-live="polite">{t(setupTextKey(method, web.started))}</p>
      <div className="agent-invite__setup-actions">
        <DownloadLink
          downloadUrl={downloadUrl}
          onStarted={web.onStarted}
          tone={web.started ? 'ghost' : 'primary'}
        />
        <Button onClick={web.onUseNetwork} tone="quiet">
          {t('agentInvite.web.setup.network')}
        </Button>
      </div>
    </section>
  );
}

function setupTextKey(method: LocalMethod, started: boolean) {
  if (method === 'mcp') {
    return started ? 'agentInvite.web.setup.startedMcp' : 'agentInvite.web.setup.mcp';
  }
  return started ? 'agentInvite.web.setup.startedCli' : 'agentInvite.web.setup.cli';
}

/** 安装包在新标签页里下，对话框留着：装好回来接着复制。 */
function DownloadLink({
  downloadUrl,
  tone,
  compact = false,
  onStarted,
}: {
  readonly downloadUrl: string;
  readonly tone: 'ghost' | 'primary';
  readonly compact?: boolean;
  readonly onStarted?: () => void;
}) {
  const { t } = useTranslation();
  const reportStep = useOnboardingStep();
  const size = compact ? 'ar-button--compact' : 'ar-button--default';
  return (
    <a
      className={`ar-button ar-button--${tone} ${size}`}
      href={downloadUrl}
      onClick={() => {
        reportStep({ step: 'desktop_download' });
        onStarted?.();
      }}
      rel="noreferrer"
      target="_blank"
    >
      <span className="ar-button__icon">
        <Download aria-hidden="true" />
      </span>
      <span>{t('agentInvite.web.download')}</span>
    </a>
  );
}
