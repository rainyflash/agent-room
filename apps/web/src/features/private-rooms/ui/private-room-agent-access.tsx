import { Button } from '@agent-room/ui-system';
import { useMutation, useQueryClient } from '@tanstack/react-query';
import {
  AlertTriangle,
  Bot,
  Check,
  ChevronDown,
  Copy,
  KeyRound,
  LoaderCircle,
  RefreshCw,
  ShieldOff,
  UserMinus,
} from 'lucide-react';
import { useState } from 'react';
import { useTranslation } from 'react-i18next';

import { useNetworkAgentGuideUrl } from '@/features/desktop/ui/network-agent-invite';
import {
  privateRoomAgentAccessQueryKey,
  privateRoomJoinCodeQueryKey,
  usePrivateRoomAgentAccess,
  usePrivateRoomJoinCode,
} from '@/features/private-rooms/data/private-room-queries';
import type {
  JoinCode,
  PrivateRoom,
  PrivateRoomAgentAccess as AgentAccessView,
  PrivateRoomAgentMember,
  PrivateRoomFailure,
  PrivateRoomGateway,
} from '@/features/private-rooms/domain/private-room';
import { PrivateRoomAgentKnocks } from '@/features/private-rooms/ui/private-room-agent-knocks';
import { PrivateRoomFailureNotice } from '@/features/private-rooms/ui/private-room-failure-notice';
import { formatDateTime } from '@/shared/i18n/formatters';
import { ok, type Result } from '@/shared/result';

/** 查口令时这两种回答不算出错：没有口令，或者有口令但服务器给不出来。 */
const expectedRevealFailures = new Set(['join_code.not_found', 'join_code.not_revealable']);

/** 服务器确认后的变化：先写进缓存，免得重新读取之前界面闪回旧状态。 */
type AccessChange = {
  readonly settle: (view: AgentAccessView) => AgentAccessView;
  /** 这次生成的口令：写进再看口令的缓存，马上显示，不用再去问一次。 */
  readonly generated?: JoinCode;
};

type AccessCommand = () => Promise<Result<AccessChange, PrivateRoomFailure>>;

/**
 * 房间设置里的「Agent 进门」：在敲门的 Agent（让它进来或不让进）；看、复制、生成、更换、停用口令；
 * 移出凭口令或经放行进来的 Agent。口令由服务器保管，管理者随时能再看（`specs/network-agents/code-first.md`）；
 * 能再看之前生成的口令看不到，要复制就换一个。
 */
export function PrivateRoomAgentAccess({
  room,
  rooms,
}: {
  readonly room: PrivateRoom;
  readonly rooms: PrivateRoomGateway;
}) {
  const { i18n, t } = useTranslation();
  const queryClient = useQueryClient();
  const access = usePrivateRoomAgentAccess(rooms, room.catalogId);
  const guide = useNetworkAgentGuideUrl();
  // 复制失败时展开给 Agent 的话，让人手动选中复制。
  const [copied, setCopied] = useState<'code' | 'failed' | 'message' | null>(null);
  const [failure, setFailure] = useState<PrivateRoomFailure | null>(null);
  const queryKey = privateRoomAgentAccessQueryKey(room.catalogId);
  const mutation = useMutation({
    mutationFn: async (command: AccessCommand) => await command(),
    onSuccess: async (result) => {
      if (!result.ok) {
        setFailure(result.error);
        return;
      }
      setFailure(null);
      const change = result.value;
      // 先记下新口令，再改口令状态，面板直接换成新口令。
      if (change.generated !== undefined) {
        queryClient.setQueryData<Result<JoinCode, PrivateRoomFailure>>(
          privateRoomJoinCodeQueryKey(room.catalogId, change.generated.createdAtUnixMs),
          ok(change.generated),
        );
        setCopied(null);
      }
      queryClient.setQueryData<Result<AgentAccessView, PrivateRoomFailure>>(queryKey, (current) =>
        current?.ok === true ? ok(change.settle(current.value)) : current,
      );
      await queryClient.invalidateQueries({ queryKey });
    },
  });

  const run = (command: AccessCommand): void => {
    setFailure(null);
    mutation.mutate(command);
  };

  const copy = (value: string, target: 'code' | 'message'): void => {
    void navigator.clipboard.writeText(value).then(
      () => {
        setCopied(target);
      },
      () => {
        setCopied('failed');
      },
    );
  };

  const generate = (): void => {
    run(async () => {
      const result = await rooms.generateJoinCode(room.catalogId);
      return result.ok
        ? ok({
            generated: result.value,
            settle: (view) => ({
              ...view,
              joinCode: { createdAtUnixMs: result.value.createdAtUnixMs },
            }),
          })
        : result;
    });
  };

  const disable = (): void => {
    run(async () => {
      const result = await rooms.disableJoinCode(room.catalogId);
      return result.ok ? ok({ settle: (view) => ({ ...view, joinCode: null }) }) : result;
    });
  };

  const remove = (agentId: string): void => {
    run(async () => {
      const result = await rooms.removeCodeAgent(room.catalogId, agentId);
      return result.ok
        ? ok({
            settle: (view) => ({
              ...view,
              agents: view.agents.map((agent) =>
                agent.agentId === agentId ? { ...agent, status: 'removed' } : agent,
              ),
            }),
          })
        : result;
    });
  };

  const view = access.data?.ok === true ? access.data.value : null;
  // 按口令状态里的生成时间去查：别人在别处换了口令，查的就是新的那个。
  const createdAt = view?.joinCode?.createdAtUnixMs ?? null;
  const revealed = usePrivateRoomJoinCode(rooms, room.catalogId, createdAt);
  const revealing = createdAt !== null && revealed.isPending;
  const visible = createdAt !== null && revealed.data?.ok === true ? revealed.data.value : null;
  const revealFailure =
    createdAt !== null &&
    revealed.data?.ok === false &&
    !expectedRevealFailures.has(revealed.data.error.code)
      ? revealed.data.error
      : null;
  const message =
    visible === null
      ? null
      : t('agentInvite.network.code.message', {
          code: visible.code,
          guide,
          room: room.name,
          roomNumber: room.catalogId,
        });
  const joined = view?.agents.filter((agent) => agent.status === 'joined') ?? [];

  return (
    <div className="private-room-agent-access">
      <div className="private-room-section-heading">
        <div>
          <h3>{t('privateRooms.knock.title')}</h3>
          <p>{t('privateRooms.knock.detail')}</p>
        </div>
      </div>
      <PrivateRoomAgentKnocks
        catalogId={room.catalogId}
        emptySpinner={false}
        emptyText={t('privateRooms.knock.none')}
        rooms={rooms}
      />

      <div className="private-room-section-heading">
        <div>
          <h3>{t('privateRooms.governance.agentAccess.agents')}</h3>
          <p>{t('privateRooms.governance.agentAccess.removeDetail')}</p>
        </div>
        {view === null ? null : <span>{joined.length}</span>}
      </div>
      {access.data?.ok === false ? <PrivateRoomFailureNotice failure={access.data.error} /> : null}
      {access.isPending ? (
        <p className="private-room-operation" role="status">
          <LoaderCircle aria-hidden="true" className="private-room-spin" />
          {t('privateRooms.governance.agentAccess.loading')}
        </p>
      ) : null}
      {view === null ? null : joined.length === 0 ? (
        <p className="private-room-agent-access__status">
          {t('privateRooms.governance.agentAccess.noAgents')}
        </p>
      ) : (
        <ol
          aria-label={t('privateRooms.governance.agentAccess.agents')}
          className="private-room-agent-access__agents"
        >
          {joined.map((agent) => (
            <CodeAgentRow
              agent={agent}
              disabled={mutation.isPending}
              key={agent.agentId}
              language={i18n.resolvedLanguage}
              onRemove={() => {
                remove(agent.agentId);
              }}
            />
          ))}
        </ol>
      )}

      <div className="private-room-section-heading">
        <div>
          <h3>{t('privateRooms.governance.agentAccess.title')}</h3>
          <p>{t('privateRooms.governance.agentAccess.detail')}</p>
        </div>
      </div>

      {view === null ? null : (
        <>
          {revealing ? (
            <p className="private-room-operation" role="status">
              <LoaderCircle aria-hidden="true" className="private-room-spin" />
              {t('privateRooms.governance.agentAccess.loading')}
            </p>
          ) : visible === null || message === null ? (
            <p className="private-room-agent-access__status">
              {view.joinCode === null
                ? t('privateRooms.governance.agentAccess.none')
                : t('privateRooms.governance.agentAccess.hidden', {
                    time: formatDateTime(view.joinCode.createdAtUnixMs, i18n.resolvedLanguage),
                  })}
            </p>
          ) : (
            <div className="private-room-join-code">
              <p className="private-room-join-code__value">
                <code>{visible.code}</code>
              </p>
              <small>
                {t('privateRooms.governance.agentAccess.createdAt', {
                  time: formatDateTime(visible.createdAtUnixMs, i18n.resolvedLanguage),
                })}
              </small>
              {copied === 'failed' ? (
                <p className="private-room-join-code__failed" role="status">
                  <AlertTriangle aria-hidden="true" />
                  {t('privateRooms.governance.agentAccess.copyFailed')}
                </p>
              ) : null}
              <div className="private-room-agent-access__actions">
                <Button
                  icon={
                    copied === 'message' ? (
                      <Check aria-hidden="true" />
                    ) : (
                      <Copy aria-hidden="true" />
                    )
                  }
                  onClick={() => {
                    copy(message, 'message');
                  }}
                  size="compact"
                  tone="primary"
                  type="button"
                >
                  {t(
                    copied === 'message'
                      ? 'privateRooms.governance.agentAccess.copied'
                      : 'privateRooms.governance.agentAccess.copyMessage',
                  )}
                </Button>
                <Button
                  icon={
                    copied === 'code' ? <Check aria-hidden="true" /> : <Copy aria-hidden="true" />
                  }
                  onClick={() => {
                    copy(visible.code, 'code');
                  }}
                  size="compact"
                  tone="quiet"
                  type="button"
                >
                  {t(
                    copied === 'code'
                      ? 'privateRooms.governance.agentAccess.copied'
                      : 'privateRooms.governance.agentAccess.copyCode',
                  )}
                </Button>
              </div>
              <details open={copied === 'failed'}>
                <summary>
                  <ChevronDown aria-hidden="true" />
                  {t('privateRooms.governance.agentAccess.preview')}
                </summary>
                <pre>{message}</pre>
              </details>
            </div>
          )}
          {revealFailure === null ? null : <PrivateRoomFailureNotice failure={revealFailure} />}

          <p className="private-room-agent-access__warning">
            <AlertTriangle aria-hidden="true" />
            {t('privateRooms.governance.agentAccess.relayWarning')}
          </p>
          <div className="private-room-agent-access__actions">
            {view.joinCode === null ? (
              <Button
                disabled={mutation.isPending}
                icon={<KeyRound aria-hidden="true" />}
                onClick={generate}
                size="compact"
                tone="primary"
                type="button"
              >
                {t('privateRooms.governance.agentAccess.generate')}
              </Button>
            ) : (
              <>
                <Button
                  disabled={mutation.isPending}
                  icon={<RefreshCw aria-hidden="true" />}
                  onClick={generate}
                  size="compact"
                  // 口令看不到时，要复制给 Agent 只能换一个：这时它是这一块要做的事。
                  tone={visible === null && !revealing ? 'primary' : 'ghost'}
                  type="button"
                >
                  {t('privateRooms.governance.agentAccess.rotate')}
                </Button>
                <Button
                  disabled={mutation.isPending}
                  icon={<ShieldOff aria-hidden="true" />}
                  onClick={disable}
                  size="compact"
                  tone="quiet"
                  type="button"
                >
                  {t('privateRooms.governance.agentAccess.disable')}
                </Button>
              </>
            )}
          </div>
          {view.joinCode === null ? null : (
            <p className="private-room-agent-access__status">
              {t('privateRooms.governance.agentAccess.rotateDetail')}
            </p>
          )}
        </>
      )}

      {failure === null ? null : <PrivateRoomFailureNotice failure={failure} />}
      {mutation.isPending ? (
        <p className="private-room-operation" role="status">
          <LoaderCircle aria-hidden="true" className="private-room-spin" />
          {t('privateRooms.governance.applying')}
        </p>
      ) : null}
    </div>
  );
}

function CodeAgentRow({
  agent,
  disabled,
  language,
  onRemove,
}: {
  readonly agent: PrivateRoomAgentMember;
  readonly disabled: boolean;
  readonly language: string | undefined;
  readonly onRemove: () => void;
}) {
  const { t } = useTranslation();
  const time = formatDateTime(agent.joinedAtUnixMs, language);
  return (
    <li>
      <div className="private-room-member__identity">
        <span>
          <Bot aria-hidden="true" />
        </span>
        <div>
          <strong>{agent.displayName}</strong>
          <small>
            {agent.ownerDisplayName === null
              ? t('privateRooms.governance.agentAccess.joinedAt', { time })
              : t('privateRooms.governance.agentAccess.ownedBy', {
                  owner: agent.ownerDisplayName,
                  time,
                })}
          </small>
        </div>
      </div>
      <Button
        aria-label={t('privateRooms.governance.agentAccess.removeAgent', {
          name: agent.displayName,
        })}
        disabled={disabled}
        icon={<UserMinus aria-hidden="true" />}
        onClick={onRemove}
        size="compact"
        tone="quiet"
        type="button"
      >
        {t('privateRooms.action.remove')}
      </Button>
    </li>
  );
}
