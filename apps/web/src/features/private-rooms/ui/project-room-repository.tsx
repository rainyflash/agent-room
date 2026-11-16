import { Banner, Button, CopyBlock, Details, Field, Spinner } from '@agent-room/ui-system';
import { useMutation, useQueryClient } from '@tanstack/react-query';
import { BadgeCheck, RefreshCw } from 'lucide-react';
import { useCallback, useEffect, useId, useState, type SubmitEvent } from 'react';
import { useTranslation } from 'react-i18next';

import {
  pendingClaimsQueryKey,
  repositoryClaimQueryKey,
  roomRepositoryMarkQueryKey,
  roomRepositoryQueryKey,
  usePendingClaims,
  useRepositoryClaim,
  useRoomRepository,
} from '@/features/private-rooms/data/project-room-queries';
import type { PrivateRoomFailure } from '@/features/private-rooms/domain/private-room';
import {
  parseRepositoryInput,
  type ProjectRoomGateway,
  type RepositoryClaim,
  type RoomRepositoryBinding,
} from '@/features/private-rooms/domain/project-rooms';
import { PrivateRoomFailureNotice } from '@/features/private-rooms/ui/private-room-failure-notice';
import { formatDateTime } from '@/shared/i18n/formatters';
import { ok, type Result } from '@/shared/result';

import './project-room-repository.css';

export type ProjectRoomRepositoryProps = {
  readonly catalogId: string;
  readonly gateway: ProjectRoomGateway;
  readonly principalId: string;
};

type Failure = PrivateRoomFailure;
type BindingResult = Result<RoomRepositoryBinding | null, Failure>;
type ClaimResult = Result<RepositoryClaim, Failure>;

/** 太快了没写多少秒时，照服务器的规矩当成一分钟。 */
const DEFAULT_RETRY_SECONDS = 60;

const CHECK_TOO_SOON = 'project_room.check_too_soon';

/**
 * 房间设置“公开”一节里的“GitHub 仓库”一组（`specs/redesign/project-rooms.md`“网页上绑已有的房间”）。只有
 * 房主、只在公开着的房间里看得到（外面那一层管）：
 * - 还没绑：填 `owner/repo`，“绑上”；
 * - 等着：在等哪个仓库、那两行（徽章、`AGENTS.md`）各一个复制、上次核对的结果，“现在核对”“不绑了”；
 * - 绑上了：哪个仓库、谁什么时候核对上的、在哪个文件里，“再核对一次”“解绑”。
 */
export function ProjectRoomRepository({
  catalogId,
  gateway,
  principalId,
}: ProjectRoomRepositoryProps) {
  const { t } = useTranslation();
  const headingId = useId();
  const binding = useRoomRepository(gateway, catalogId);

  let content;
  if (binding.isPending) {
    content = (
      <p className="room-listing__note" role="status">
        <Spinner />
        {t('projectRooms.loading')}
      </p>
    );
  } else if (binding.isError || !binding.data.ok) {
    content = (
      <>
        {binding.data?.ok === false ? (
          <PrivateRoomFailureNotice failure={binding.data.error} />
        ) : null}
        <Button
          className="room-repository__retry"
          icon={<RefreshCw aria-hidden="true" />}
          onClick={() => void binding.refetch()}
          size="compact"
          tone="quiet"
        >
          {t('projectRooms.retry')}
        </Button>
      </>
    );
  } else if (binding.data.value === null) {
    content = <ClaimSection catalogId={catalogId} gateway={gateway} />;
  } else {
    content = (
      <BoundRepository
        binding={binding.data.value}
        catalogId={catalogId}
        gateway={gateway}
        principalId={principalId}
      />
    );
  }

  return (
    <section aria-labelledby={headingId} className="room-repository">
      <h3 id={headingId}>{t('projectRooms.title')}</h3>
      <p className="room-listing__note">{t('projectRooms.detail')}</p>
      {content}
    </section>
  );
}

type ClaimSectionProps = {
  readonly catalogId: string;
  readonly gateway: ProjectRoomGateway;
};

/**
 * 这间房在等的认领问服务器（自己在等的认领里要绑这间房的那一个，一间房同时只能有一个）：换个浏览器、
 * 换台电脑也接得上。没有就给输入框。
 */
function ClaimSection({ catalogId, gateway }: ClaimSectionProps) {
  const { t } = useTranslation();
  const claims = usePendingClaims(gateway);
  if (claims.isPending) {
    return <ClaimLoading />;
  }
  if (claims.isError || !claims.data.ok) {
    return (
      <>
        {claims.data?.ok === false ? (
          <ProjectRoomFailureNotice failure={claims.data.error} />
        ) : null}
        <Button
          className="room-repository__retry"
          icon={<RefreshCw aria-hidden="true" />}
          onClick={() => void claims.refetch()}
          size="compact"
          tone="quiet"
        >
          {t('projectRooms.retry')}
        </Button>
      </>
    );
  }
  const waiting = claims.data.value.find(
    (claim) => claim.catalogId === catalogId && claim.claimId !== null,
  );
  return (
    <RoomClaim catalogId={catalogId} gateway={gateway} initialClaimId={waiting?.claimId ?? null} />
  );
}

function RoomClaim({
  catalogId,
  gateway,
  initialClaimId,
}: ClaimSectionProps & { readonly initialClaimId: string | null }) {
  const { t } = useTranslation();
  const queryClient = useQueryClient();
  const [claimId, setClaimId] = useState(initialClaimId);
  // 认领没了（认上了、作废了、房间不能绑了）时，回到输入框上面说一句为什么。
  const [gone, setGone] = useState<Failure | null>(null);
  const claim = useRepositoryClaim(gateway, claimId);

  const settle = useCallback(
    (reason: Failure | null) => {
      setClaimId(null);
      setGone(reason);
      void queryClient.invalidateQueries({ queryKey: pendingClaimsQueryKey });
      // 可能已经绑上了：再问一次这间房绑着哪个（勾挂在它下面，一起重问）。
      void queryClient.invalidateQueries({ queryKey: roomRepositoryQueryKey(catalogId) });
    },
    [catalogId, queryClient],
  );

  const data = claim.data;
  const finished =
    data !== undefined &&
    ((data.ok && data.value.status === 'bound') ||
      (!data.ok && data.error.code === 'project_room.not_found'));
  useEffect(() => {
    if (!finished) return;
    settle(data.ok ? null : data.error);
  }, [data, finished, settle]);

  if (claimId === null) {
    return (
      <ClaimForm
        catalogId={catalogId}
        gateway={gateway}
        gone={gone}
        onClaimed={(next) => {
          void queryClient.invalidateQueries({ queryKey: pendingClaimsQueryKey });
          setGone(null);
          setClaimId(next);
        }}
        onBound={() => {
          settle(null);
        }}
      />
    );
  }
  if (claim.isPending || finished) {
    return <ClaimLoading />;
  }
  if (claim.isError || !claim.data.ok) {
    return (
      <>
        {claim.data?.ok === false ? <ProjectRoomFailureNotice failure={claim.data.error} /> : null}
        <Button
          className="room-repository__retry"
          icon={<RefreshCw aria-hidden="true" />}
          onClick={() => void claim.refetch()}
          size="compact"
          tone="quiet"
        >
          {t('projectRooms.retry')}
        </Button>
      </>
    );
  }
  return <PendingClaim claim={claim.data.value} gateway={gateway} onGone={settle} />;
}

function ClaimLoading() {
  const { t } = useTranslation();
  return (
    <p className="room-listing__note" role="status">
      <Spinner />
      {t('projectRooms.loading')}
    </p>
  );
}

function ClaimForm({
  catalogId,
  gateway,
  gone,
  onBound,
  onClaimed,
}: {
  readonly catalogId: string;
  readonly gateway: ProjectRoomGateway;
  readonly gone: Failure | null;
  readonly onBound: () => void;
  readonly onClaimed: (claimId: string) => void;
}) {
  const { t } = useTranslation();
  const queryClient = useQueryClient();
  const [text, setText] = useState('');
  const [invalid, setInvalid] = useState(false);
  const claim = useMutation({
    mutationFn: async (repository: string) => await gateway.claim({ catalogId, repository }),
    onSuccess: (result) => {
      if (!result.ok) return;
      if (result.value.status === 'pending' && result.value.claimId !== null) {
        queryClient.setQueryData<ClaimResult>(
          repositoryClaimQueryKey(result.value.claimId),
          result,
        );
        onClaimed(result.value.claimId);
      } else {
        onBound();
      }
    },
  });
  const submit = (event: SubmitEvent<HTMLFormElement>): void => {
    event.preventDefault();
    const repository = parseRepositoryInput(text);
    setInvalid(repository === null);
    claim.reset();
    if (repository !== null) claim.mutate(repository);
  };
  const failure = claim.data?.ok === false ? claim.data.error : null;
  return (
    <>
      {gone === null ? null : <ProjectRoomFailureNotice failure={gone} />}
      <form className="room-repository__form" noValidate onSubmit={submit}>
        <Field
          error={invalid ? t('projectRooms.input.invalid') : undefined}
          hint={t('projectRooms.input.hint')}
          label={t('projectRooms.input.label')}
        >
          <input
            autoCapitalize="off"
            autoComplete="off"
            disabled={claim.isPending}
            maxLength={200}
            onChange={(event) => {
              setText(event.target.value);
              setInvalid(false);
            }}
            placeholder="owner/repo"
            spellCheck={false}
            value={text}
          />
        </Field>
        <Button disabled={claim.isPending} tone="primary" type="submit">
          {claim.isPending ? t('projectRooms.linking') : t('projectRooms.link')}
        </Button>
      </form>
      {failure === null ? null : <ProjectRoomFailureNotice failure={failure} />}
    </>
  );
}

function PendingClaim({
  claim,
  gateway,
  onGone,
}: {
  readonly claim: RepositoryClaim;
  readonly gateway: ProjectRoomGateway;
  readonly onGone: (reason: Failure | null) => void;
}) {
  const { t, i18n } = useTranslation();
  const queryClient = useQueryClient();
  const [waitSeconds, waitFor] = useCountdown();
  const claimId = claim.claimId ?? '';
  const check = useMutation({
    mutationFn: async () => await gateway.checkClaim(claimId),
    onSuccess: (result) => {
      if (result.ok) {
        queryClient.setQueryData<ClaimResult>(repositoryClaimQueryKey(claimId), result);
        if (result.value.status === 'bound') onGone(null);
        return;
      }
      if (result.error.code === CHECK_TOO_SOON) {
        waitFor(result.error.retryAfterSeconds ?? DEFAULT_RETRY_SECONDS);
      } else if (gone(result.error)) {
        onGone(result.error);
      }
    },
  });
  const cancel = useMutation({
    mutationFn: async () => await gateway.cancelClaim(claimId),
    onSuccess: (result) => {
      if (result.ok || result.error.code === 'project_room.not_found') {
        queryClient.removeQueries({ queryKey: repositoryClaimQueryKey(claimId) });
        onGone(null);
      }
    },
  });
  const failure =
    check.data?.ok === false && check.data.error.code !== CHECK_TOO_SOON
      ? check.data.error
      : cancel.data?.ok === false
        ? cancel.data.error
        : null;
  const busy = check.isPending || cancel.isPending;
  return (
    <div className="room-repository__body">
      <p className="room-repository__state">
        {t('projectRooms.pending.waiting', { repository: claim.repository })}
      </p>
      <p className="room-listing__note">{t('projectRooms.pending.instructions')}</p>
      <RepositoryLine label={t('projectRooms.badge.label')} text={claim.badgeMarkdown} />
      <RepositoryLine label={t('projectRooms.agents.label')} text={claim.agentsLine} />
      <p className="room-listing__note">{t('projectRooms.pending.afterPush')}</p>
      <p aria-live="polite" className="room-repository__result">
        {claim.lastCheck === null
          ? t('projectRooms.pending.neverChecked')
          : t('projectRooms.pending.lastCheck', {
              result: t(`projectRooms.result.${claim.lastCheck.result}`, { file: '' }),
              time: formatDateTime(claim.lastCheck.atUnixMs, i18n.resolvedLanguage),
            })}
      </p>
      {waitSeconds > 0 ? (
        <p className="room-listing__note" role="status">
          {t('projectRooms.checkTooSoon', { count: waitSeconds })}
        </p>
      ) : null}
      <div className="room-listing__actions">
        <Button
          disabled={busy || waitSeconds > 0}
          icon={<RefreshCw aria-hidden="true" />}
          onClick={() => {
            cancel.reset();
            check.mutate();
          }}
          size="compact"
          tone="primary"
        >
          {check.isPending ? t('projectRooms.checking') : t('projectRooms.checkNow')}
        </Button>
        <Button
          disabled={busy}
          onClick={() => {
            check.reset();
            cancel.mutate();
          }}
          size="compact"
          tone="ghost"
        >
          {cancel.isPending ? t('projectRooms.cancelling') : t('projectRooms.cancel')}
        </Button>
      </div>
      {failure === null ? null : <ProjectRoomFailureNotice failure={failure} />}
    </div>
  );
}

/** 认领在服务器上已经没了：认上了删掉、作废了，或者房间变了不能再绑。 */
function gone(failure: Failure): boolean {
  return (
    failure.code === 'project_room.not_found' || failure.code === 'project_room.room_not_eligible'
  );
}

function RepositoryLine({ label, text }: { readonly label: string; readonly text: string }) {
  const { t } = useTranslation();
  return (
    <div className="room-repository__line">
      <span className="room-repository__label">{label}</span>
      <CopyBlock
        copiedLabel={t('projectRooms.copied')}
        copyLabel={t('projectRooms.copy')}
        failedLabel={t('projectRooms.copyFailed')}
        size="compact"
        text={text}
        textLabel={label}
        tone="quiet"
      />
    </div>
  );
}

function BoundRepository({
  binding,
  catalogId,
  gateway,
  principalId,
}: ProjectRoomRepositoryProps & { readonly binding: RoomRepositoryBinding }) {
  const { t, i18n } = useTranslation();
  const queryClient = useQueryClient();
  const [confirming, setConfirming] = useState(false);
  const [waitSeconds, waitFor] = useCountdown();
  const check = useMutation({
    mutationFn: async () => await gateway.checkBinding(catalogId),
    onSuccess: (result) => {
      if (result.ok) {
        queryClient.setQueryData<BindingResult>(
          roomRepositoryQueryKey(catalogId),
          ok(result.value.binding),
        );
        void queryClient.invalidateQueries({ queryKey: roomRepositoryMarkQueryKey(catalogId) });
      } else if (result.error.code === CHECK_TOO_SOON) {
        waitFor(result.error.retryAfterSeconds ?? DEFAULT_RETRY_SECONDS);
      }
    },
  });
  const unbind = useMutation({
    mutationFn: async () => await gateway.unbind(catalogId),
    onSuccess: (result) => {
      setConfirming(false);
      if (!result.ok) return;
      queryClient.setQueryData<BindingResult>(roomRepositoryQueryKey(catalogId), ok(null));
      // 勾没了：房间名旁边那一份跟着重问。
      void queryClient.invalidateQueries({ queryKey: roomRepositoryMarkQueryKey(catalogId) });
    },
  });
  const lastCheck = check.data?.ok === true ? check.data.value.lastCheck : null;
  const checkFailure =
    check.data?.ok === false && check.data.error.code !== CHECK_TOO_SOON ? check.data.error : null;
  const unbindFailure = unbind.data?.ok === false ? unbind.data.error : null;
  const busy = check.isPending || unbind.isPending;
  const you = binding.verifiedByAccountId === principalId;
  return (
    <div className="room-repository__body">
      <p className="room-repository__state">
        <BadgeCheck aria-hidden="true" className="room-repository__mark" />
        <a
          aria-label={t('projectRooms.bound.openRepository', { repository: binding.repository })}
          href={binding.repositoryUrl}
          rel="noopener noreferrer"
          target="_blank"
        >
          {binding.repository}
        </a>
      </p>
      <p className="room-listing__note">
        {t('projectRooms.bound.verified', {
          file: binding.verifiedFile,
          time: formatDateTime(binding.verifiedAtUnixMs, i18n.resolvedLanguage),
          who: t(you ? 'projectRooms.bound.you' : 'projectRooms.bound.someoneElse'),
        })}
      </p>
      {you ? null : (
        <Details summary={t('projectRooms.bound.details')}>
          {t('projectRooms.bound.accountId')} <code>{binding.verifiedByAccountId}</code>
        </Details>
      )}
      {lastCheck === null ? null : (
        <p aria-live="polite" className="room-repository__result">
          {t('projectRooms.bound.lastCheck', {
            result: t(`projectRooms.result.${lastCheck.result}`, { file: binding.verifiedFile }),
            time: formatDateTime(lastCheck.atUnixMs, i18n.resolvedLanguage),
          })}
          {lastCheck.result === 'found' ? null : ` ${t('projectRooms.bound.stillLinked')}`}
        </p>
      )}
      {waitSeconds > 0 ? (
        <p className="room-listing__note" role="status">
          {t('projectRooms.checkTooSoon', { count: waitSeconds })}
        </p>
      ) : null}
      {confirming ? (
        <Banner
          action={
            <div className="room-listing__actions">
              <Button
                disabled={unbind.isPending}
                onClick={() => {
                  unbind.mutate();
                }}
                size="compact"
                tone="alert"
              >
                {unbind.isPending
                  ? t('projectRooms.unbinding')
                  : t('projectRooms.unbind.confirmYes')}
              </Button>
              <Button
                disabled={unbind.isPending}
                onClick={() => {
                  setConfirming(false);
                }}
                size="compact"
                tone="ghost"
              >
                {t('projectRooms.unbind.cancel')}
              </Button>
            </div>
          }
          tone="warning"
        >
          {t('projectRooms.unbind.confirm')}
        </Banner>
      ) : (
        <div className="room-listing__actions">
          <Button
            disabled={busy || waitSeconds > 0}
            icon={<RefreshCw aria-hidden="true" />}
            onClick={() => {
              unbind.reset();
              check.mutate();
            }}
            size="compact"
            tone="quiet"
          >
            {check.isPending ? t('projectRooms.checking') : t('projectRooms.checkAgain')}
          </Button>
          <Button
            disabled={busy}
            onClick={() => {
              check.reset();
              unbind.reset();
              setConfirming(true);
            }}
            size="compact"
            tone="ghost"
          >
            {t('projectRooms.unbind')}
          </Button>
        </div>
      )}
      {checkFailure === null ? null : <ProjectRoomFailureNotice failure={checkFailure} />}
      {unbindFailure === null ? null : <ProjectRoomFailureNotice failure={unbindFailure} />}
    </div>
  );
}

/** 有自己说法的错误码：一句话说清楚，错误码和关联 ID 收进详情。别的照私人房间的通用说法。 */
const failureMessages = {
  'private_room.public_limit_reached': 'projectRooms.unbind.limitReached',
  'project_room.conflict': 'projectRooms.error.conflict',
  'project_room.invalid_repository': 'projectRooms.error.invalid_repository',
  'project_room.not_found': 'projectRooms.error.claim_gone',
  'project_room.repository_taken': 'projectRooms.error.repository_taken',
  'project_room.room_not_eligible': 'projectRooms.error.room_not_eligible',
  'project_room.too_many_claims': 'projectRooms.error.too_many_claims',
  'room_listing.repository_bound': 'projectRooms.error.already_bound',
} as const;

function ProjectRoomFailureNotice({ failure }: { readonly failure: Failure }) {
  const { t } = useTranslation();
  if (!Object.hasOwn(failureMessages, failure.code)) {
    return <PrivateRoomFailureNotice failure={failure} />;
  }
  const message = failureMessages[failure.code as keyof typeof failureMessages];
  return (
    <Banner tone="warning">
      <p>{t(message)}</p>
      <Details summary={t('projectRooms.bound.details')}>
        <code>{failure.code}</code>
        {failure.correlationId === undefined ? null : (
          <>
            {' · '}
            <code>{failure.correlationId}</code>
          </>
        )}
      </Details>
    </Banner>
  );
}

/** 服务器说多少秒以后再来：按钮先灰着，每秒数一下。 */
function useCountdown(): readonly [number, (seconds: number) => void] {
  const [until, setUntil] = useState<number | null>(null);
  const [now, setNow] = useState(() => Date.now());
  useEffect(() => {
    if (until === null) return;
    const timer = globalThis.setInterval(() => {
      const current = Date.now();
      setNow(current);
      if (current >= until) setUntil(null);
    }, 1_000);
    return () => {
      globalThis.clearInterval(timer);
    };
  }, [until]);
  const start = useCallback((seconds: number) => {
    const at = Date.now();
    setNow(at);
    setUntil(at + Math.max(1, seconds) * 1_000);
  }, []);
  const left = until === null ? 0 : Math.max(0, Math.ceil((until - now) / 1_000));
  return [left, start] as const;
}
