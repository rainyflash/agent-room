import { Banner, Button, Dialog } from '@agent-room/ui-system';
import { useState } from 'react';
import { useTranslation } from 'react-i18next';

import { agentPlace } from '@/features/workspace/domain/agent-card';
import type { FleetAgent } from '@/features/workspace/domain/agent-fleet';
import type { StaleAgent } from '@/features/workspace/domain/stale-agents';
import { placeText } from '@/features/workspace/ui/agent-card-list';
import { formatRelativeTime } from '@/features/workspace/ui/workspace-format';

const REAUTHENTICATION_REQUIRED = 'authentication.reauthentication_required';

type Progress = { readonly done: number; readonly total: number };

/**
 * “30 天没上线的”一键删除（维护者 2026-11-01 定）：列出每个，默认都勾上，要留着的取消勾选，确认后
 * 一个个删（先撤销它每个还在用的连接，再删它）。没删成的留在列表里、还勾着，可以再点一次。
 */
export function StaleAgentCleanupDialog({
  agents,
  now,
  onClose,
  onDone,
  onRetire,
  reauthentication,
}: {
  readonly agents: readonly StaleAgent[];
  readonly now: number;
  readonly onClose: () => void;
  /** 删完一轮以后刷新列表。 */
  readonly onDone: () => Promise<void>;
  /** 删一个，成了回 `null`，没成回错误码。 */
  readonly onRetire: (agent: FleetAgent) => Promise<string | null>;
  readonly reauthentication: {
    readonly recentlyAuthenticated: boolean;
    readonly signInAgain: () => void;
  };
}) {
  const { i18n, t } = useTranslation();
  const [kept, setKept] = useState<ReadonlySet<string>>(() => new Set());
  const [progress, setProgress] = useState<Progress | null>(null);
  const [failed, setFailed] = useState(0);
  const [signInRejected, setSignInRejected] = useState(false);
  const needsSignIn = signInRejected || !reauthentication.recentlyAuthenticated;
  const chosen = agents.filter((stale) => !kept.has(stale.entry.agent.agentId));
  const running = progress !== null;

  const retire = async (): Promise<void> => {
    const targets = chosen;
    let failures = 0;
    let rejected = false;
    setFailed(0);
    setProgress({ done: 0, total: targets.length });
    for (const [index, stale] of targets.entries()) {
      const code = await onRetire(stale.entry);
      if (code === REAUTHENTICATION_REQUIRED) {
        rejected = true;
        break;
      }
      if (code !== null) failures += 1;
      setProgress({ done: index + 1, total: targets.length });
    }
    await onDone();
    setProgress(null);
    setFailed(failures);
    setSignInRejected(rejected);
    if (failures === 0 && !rejected) onClose();
  };

  return (
    <Dialog
      className="stale-agent-cleanup"
      closeLabel={t('workspace.stale.close')}
      description={t('workspace.stale.detail')}
      footer={
        <>
          <Button disabled={running} onClick={onClose} size="compact" tone="quiet">
            {t('workspace.delete.cancel')}
          </Button>
          {needsSignIn ? (
            <Button onClick={reauthentication.signInAgain} size="compact" tone="primary">
              {t('workspace.delete.signInAgain')}
            </Button>
          ) : (
            <Button
              disabled={running || chosen.length === 0}
              onClick={() => void retire()}
              size="compact"
              tone="alert"
            >
              {progress === null
                ? t('workspace.stale.confirm', { count: chosen.length })
                : t('workspace.stale.progress', progress)}
            </Button>
          )}
        </>
      }
      onClose={() => {
        if (!running) onClose();
      }}
      title={t('workspace.stale.title')}
    >
      {needsSignIn ? (
        <Banner icon={null} role="status" title={t('workspace.delete.signIn')} tone="info" />
      ) : null}
      {failed > 0 ? (
        <p className="stale-agent-cleanup__failure" role="alert">
          {t('workspace.stale.failed', { count: failed })}
        </p>
      ) : null}
      <ul className="stale-agent-cleanup__list">
        {agents.map((stale) => {
          const agentId = stale.entry.agent.agentId;
          const place = agentPlace(stale.entry);
          return (
            <li key={agentId}>
              <label>
                <input
                  checked={!kept.has(agentId)}
                  disabled={running}
                  onChange={(event) => {
                    const keep = !event.target.checked;
                    setKept((current) => {
                      const next = new Set(current);
                      if (keep) next.add(agentId);
                      else next.delete(agentId);
                      return next;
                    });
                  }}
                  type="checkbox"
                />
                <span className="stale-agent-cleanup__identity">
                  <strong>{stale.entry.agent.displayName}</strong>
                  <span className="stale-agent-cleanup__metadata">
                    <time dateTime={new Date(stale.lastOnlineAtUnixMs).toISOString()}>
                      {t('workspace.activity.lastSeen', {
                        time: formatRelativeTime(
                          stale.lastOnlineAtUnixMs,
                          now,
                          i18n.resolvedLanguage,
                        ),
                      })}
                    </time>
                    {place.kind === 'none' ? null : <span>{placeText(t, place)}</span>}
                  </span>
                </span>
              </label>
            </li>
          );
        })}
      </ul>
    </Dialog>
  );
}
