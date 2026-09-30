import { CopyBlock } from '@agent-room/ui-system';
import { useMemo } from 'react';
import { useTranslation } from 'react-i18next';

import type { ManualHostConfiguration } from '../domain/desktop-runtime';
import { serializeManualHostConfiguration } from '../domain/manual-host-configuration';
import './agent-invite-dialog.css';

/**
 * 通用的 MCP 配置：任何支持 MCP 的 Agent 工具都按这段 JSON 添加本机的 Agent Room 服务，
 * 不按具体的 Agent 应用区分。
 */
export function McpConfiguration({
  configuration,
}: {
  readonly configuration: ManualHostConfiguration;
}) {
  const { t } = useTranslation();
  const serialized = useMemo(
    () => serializeManualHostConfiguration(configuration),
    [configuration],
  );
  return (
    <div className="mcp-configuration">
      <p>{t('agentInvite.host.otherHint')}</p>
      <CopyBlock
        copiedLabel={t('agentInvite.host.copiedJson')}
        copyLabel={t('agentInvite.host.copyJson')}
        failedLabel={t('agentInvite.message.failed')}
        size="compact"
        text={serialized}
        textLabel={t('agentInvite.mode.mcp')}
        tone="ghost"
      />
    </div>
  );
}
