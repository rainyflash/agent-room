import { Button } from '@agent-room/ui-system';
import { Check, Copy } from 'lucide-react';
import { useMemo, useState } from 'react';
import { useTranslation } from 'react-i18next';

import type { ManualHostConfiguration } from '../domain/desktop-runtime';
import { serializeManualHostConfiguration } from '../domain/manual-host-configuration';
import './mcp-configuration.css';

type CopyState = 'idle' | 'copied' | 'failed';

/**
 * 通用的 MCP 配置：任何支持 MCP 的 Agent 工具都按这段 JSON 添加本机的 Agent Room 服务。
 * 接入面板、「本机 Agent」设置和首次使用页都用这一份，不按具体的 Agent 应用区分。
 */
export function McpConfiguration({
  configuration,
}: {
  readonly configuration: ManualHostConfiguration;
}) {
  const { t } = useTranslation();
  const [copyState, setCopyState] = useState<CopyState>('idle');
  const serialized = useMemo(
    () => serializeManualHostConfiguration(configuration),
    [configuration],
  );
  const copy = async (): Promise<void> => {
    try {
      await navigator.clipboard.writeText(serialized);
      setCopyState('copied');
    } catch {
      setCopyState('failed');
    }
  };
  return (
    <div className="mcp-configuration">
      <p>{t('agentInvite.host.otherHint')}</p>
      <pre>{serialized}</pre>
      <Button
        icon={copyState === 'copied' ? <Check aria-hidden="true" /> : <Copy aria-hidden="true" />}
        onClick={() => void copy()}
        size="compact"
        tone={copyState === 'failed' ? 'alert' : 'ghost'}
      >
        {t(copyState === 'copied' ? 'agentInvite.host.copiedJson' : 'agentInvite.host.copyJson')}
      </Button>
    </div>
  );
}
