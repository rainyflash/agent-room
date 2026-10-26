import { Button } from '@agent-room/ui-system';
import { useMutation, useQuery, useQueryClient } from '@tanstack/react-query';
import { CircleAlert, LoaderCircle } from 'lucide-react';
import { useTranslation } from 'react-i18next';

import {
  blockedAgentListQueryKey,
  directSessionListQueryKey,
} from '@/features/direct-sessions/data/direct-session-queries';
import type {
  BlockedAgent,
  DirectSessionFailure,
} from '@/features/direct-sessions/domain/direct-session';
import type { Result } from '@/shared/result';

/** 读屏蔽、解除屏蔽；真实的是 `DirectSessionCoordinator`。 */
export type BlockedAgentsSource = {
  listBlocked(): Promise<Result<readonly BlockedAgent[], DirectSessionFailure>>;
  unblock(blocked: BlockedAgent): Promise<Result<void, DirectSessionFailure>>;
};

/**
 * “设置 → 安全”里屏蔽的 Agent：一行一个，右边“解除屏蔽”。在房间里屏蔽的、在别的设备上屏蔽的都在这里，
 * Agent 离开了房间、被删了也能在这里解除。
 */
export function BlockedAgentsPanel({ source }: { readonly source: BlockedAgentsSource }) {
  const { i18n, t } = useTranslation();
  const queryClient = useQueryClient();
  const blocked = useQuery({
    queryKey: blockedAgentListQueryKey,
    queryFn: async () => await source.listBlocked(),
    networkMode: 'always',
    retry: false,
    staleTime: 3_000,
  });
  const unblock = useMutation({
    mutationFn: async (entry: BlockedAgent) => await source.unblock(entry),
    onSettled: async () => {
      await Promise.all([
        queryClient.invalidateQueries({ queryKey: blockedAgentListQueryKey }),
        queryClient.invalidateQueries({ queryKey: directSessionListQueryKey }),
      ]);
    },
  });
  const listFailure =
    blocked.data?.ok === false
      ? blocked.data.error
      : blocked.isError
        ? { code: 'direct_session.query_failed', retryable: true }
        : null;
  const entries = blocked.data?.ok === true ? blocked.data.value : [];
  const unblockFailure = unblock.data?.ok === false ? unblock.data.error : null;
  const pendingId = unblock.isPending ? unblock.variables.matrixUserId : null;
  const formatDate = new Intl.DateTimeFormat(i18n.resolvedLanguage, { dateStyle: 'medium' });

  return (
    <section
      aria-labelledby="security-blocks-title"
      className="security-blocks"
      id="security-blocks"
    >
      <header className="security-section-heading">
        <div>
          <h3 id="security-blocks-title">{t('security.blocks.title')}</h3>
          <p>{t('security.blocks.detail')}</p>
        </div>
      </header>
      {blocked.isPending ? (
        <p className="security-blocks__boundary" role="status">
          <LoaderCircle aria-hidden="true" className="security-spin" />
          {t('security.blocks.loading')}
        </p>
      ) : listFailure !== null ? (
        <div className="security-blocks__boundary" role="alert">
          <CircleAlert aria-hidden="true" />
          <span>{t('security.blocks.loadFailed', { code: listFailure.code })}</span>
          <Button onClick={() => void blocked.refetch()} size="compact" tone="quiet">
            {t('security.blocks.retry')}
          </Button>
        </div>
      ) : entries.length === 0 ? (
        <p className="security-blocks__boundary">{t('security.blocks.empty')}</p>
      ) : (
        <ol className="security-access-list">
          {entries.map((entry) => (
            <li key={entry.matrixUserId}>
              <div className="security-access-list__identity">
                {/* 没名字的是这台设备没见过的；只好写 Matrix ID，不然认不出是谁。 */}
                <strong>{entry.displayName ?? entry.matrixUserId}</strong>
              </div>
              {entry.blockedAtUnixMs === null ? null : (
                <div className="security-access-list__metadata">
                  <time dateTime={new Date(entry.blockedAtUnixMs).toISOString()}>
                    {t('security.blocks.blockedAt', {
                      date: formatDate.format(entry.blockedAtUnixMs),
                    })}
                  </time>
                </div>
              )}
              <Button
                disabled={pendingId !== null}
                onClick={() => {
                  unblock.mutate(entry);
                }}
                size="compact"
                tone="ghost"
              >
                {pendingId === entry.matrixUserId
                  ? t('security.blocks.unblocking')
                  : t('security.blocks.unblock')}
              </Button>
            </li>
          ))}
        </ol>
      )}
      {unblockFailure === null ? null : (
        <p className="security-blocks__failure" role="alert">
          {t('security.blocks.unblockFailed', { code: unblockFailure.code })}
        </p>
      )}
    </section>
  );
}
