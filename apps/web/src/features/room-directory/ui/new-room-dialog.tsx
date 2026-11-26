import { Banner, Button, Details, Dialog, Field, Spinner } from '@agent-room/ui-system';
import { useQueryClient } from '@tanstack/react-query';
import { DoorOpen } from 'lucide-react';
import { useId, useMemo, useRef, useState } from 'react';
import { useTranslation } from 'react-i18next';

import { useAppServices } from '@/app/app-services';
import { cliInvocation } from '@/features/desktop/domain/cli-invitation';
import { useOptionalDesktopRuntimeController } from '@/features/desktop/ui/desktop-runtime-provider';
import { privateRoomListQueryOptions } from '@/features/private-rooms/data/private-room-queries';
import { roomLiveViewQueryKey } from '@/features/private-rooms/data/room-live-view-queries';
import {
  permissions,
  type CreatePrivateRoomInput,
  type PrivateRoom,
  type PrivateRoomFailure,
  type PrivateRoomPermissions,
} from '@/features/private-rooms/domain/private-room';
import { parseInvitationList } from '@/features/private-rooms/domain/private-room-invitations';
import { DEFAULT_INVITE_PERMISSIONS } from '@/features/private-rooms/domain/private-room-invite-link';
import { PrivateRoomCapabilityEditor } from '@/features/private-rooms/ui/private-room-capability-editor';
import {
  RoomListingChoices,
  type RoomListingChoice,
} from '@/features/private-rooms/ui/room-listing-choices';
import {
  NewProjectRoom,
  type ProjectRoomSource,
} from '@/features/room-directory/ui/new-project-room';
import { useRoomCommands } from '@/features/room-directory/ui/use-rooms';
import { useOptionalSession } from '@/features/session/ui/session-provider';
import { BrowserUuidV7Factory } from '@/shared/ids/browser-uuid-v7-factory';
import type { Result } from '@/shared/result';

const RETENTION_CHOICES = [7, 30, 90, 365] as const;

/**
 * 服务器说这份填写本身不行（400）：什么都没建，再发同一个请求也只会再被拒。放开表单让人改，
 * 下次是一个新请求（agent-room-ops#57）。
 */
const REJECTED_INPUT = new Set([
  'private_room.invalid_request',
  'private_room.invalid_creation_body',
  // 已经有一间公开房间：改成私密或者先去改另一间，再建。
  'private_room.public_limit_reached',
]);

/** 新建房间：连着服务，建好后直接进去。从发现页打开时一开始就选着公开。 */
export function NewRoomDialog({
  initiallyListed = false,
  onClose,
}: {
  readonly initiallyListed?: boolean;
  readonly onClose: () => void;
}) {
  const commands = useRoomCommands();
  const { privateRooms, projectRooms, roomLiveViews } = useAppServices();
  const queryClient = useQueryClient();
  const principalId = useOptionalSession()?.snapshot.context.principal?.principalId;
  const desktop = useOptionalDesktopRuntimeController();
  const onDesktop = desktop?.available === true;
  const platform = desktop?.snapshot?.platform ?? 'unknown';
  const cliConfiguration = onDesktop ? desktop.snapshot?.cliConfiguration : undefined;
  const projectRoom = useMemo<ProjectRoomSource | undefined>(
    () =>
      projectRooms === undefined
        ? undefined
        : {
            invocation: cliInvocation(cliConfiguration, platform),
            locate: !onDesktop,
            claims: async () => await projectRooms.claims(),
            // 认上以后控制面替这个账户建好房间，名字就是仓库名：重读房间列表找它。
            findRoom: async (repository) => {
              const rooms = await queryClient.query({
                ...privateRoomListQueryOptions(privateRooms, principalId),
                staleTime: 0,
              });
              if (!rooms.ok) return null;
              const wanted = repository.toLowerCase();
              return (
                rooms.value.find(
                  (room) =>
                    room.status === 'active' &&
                    room.ownerPrincipalId === principalId &&
                    room.name.toLowerCase() === wanted,
                ) ?? null
              );
            },
          },
    [cliConfiguration, onDesktop, platform, principalId, privateRooms, projectRooms, queryClient],
  );
  return (
    <NewRoomForm
      initiallyListed={initiallyListed}
      onClose={onClose}
      onCreate={commands.create}
      onEnter={commands.show}
      projectRoom={projectRoom}
      onTurnOnLiveView={
        roomLiveViews === undefined
          ? undefined
          : async (catalogId) => {
              const result = await roomLiveViews.updateLiveView(catalogId, true);
              // 进去以后顶栏那一行就有了，不用再问一次。
              if (result.ok) queryClient.setQueryData(roomLiveViewQueryKey(catalogId), result);
              return result;
            }
      }
    />
  );
}

export type NewRoomFormProps = {
  readonly initiallyListed?: boolean;
  readonly onClose: () => void;
  /** 建好、加入，不跳过去。同一次填写的重试沿用同一个请求 ID，服务端不会再建一个房间。 */
  readonly onCreate: (
    requestId: string,
    input: CreatePrivateRoomInput,
  ) => Promise<Result<PrivateRoom, PrivateRoomFailure>>;
  /** 跳进建好的房间。 */
  readonly onEnter: (room: PrivateRoom) => Promise<void>;
  /**
   * 建好以后打开分享页的实况（`specs/redesign/project-rooms.md`“分享页放实况”）：建房的请求里不带它，建好再
   * `PUT` 一次。没有这项服务时不出那个勾。
   */
  readonly onTurnOnLiveView?:
    ((catalogId: string) => Promise<Result<unknown, PrivateRoomFailure>>) | undefined;
  /**
   * 选公开时多一个“给一个 GitHub 项目建房”（`specs/redesign/project-front-door.md`）。没有认领这项服务时
   * 没有这个选项。
   */
  readonly projectRoom?: ProjectRoomSource | undefined;
};

/**
 * 一屏建好房间：名字、用途、私密还是公开（`specs/public-rooms/design.md`“网页”）；保留多久、现在就
 * 邀请谁放在“更多选项”里，不填就用默认值。选公开就说清楚不加密，新成员的权限和网络 Agent 先用默认的，
 * 以后在房间设置的“公开”里改。公开、直接进的房间多一个“分享页放实况”的勾，默认勾上；建好以后再打开它，
 * 没开成房间照样在，说一句、给“进房间”。选了“给一个 GitHub 项目建房”时名字、用途、怎么进和更多选项都
 * 收起来，换成发给 Agent 的一段话：房间由服务器认上仓库以后建，这里不建；建好了底下给“进房间”。
 */
export function NewRoomForm({
  initiallyListed = false,
  onClose,
  onCreate,
  onEnter,
  onTurnOnLiveView,
  projectRoom,
}: NewRoomFormProps) {
  const { t } = useTranslation();
  const formId = useId();
  const identifiers = useMemo(() => new BrowserUuidV7Factory(), []);
  const [name, setName] = useState('');
  const [purpose, setPurpose] = useState('');
  const [listing, setListing] = useState<RoomListingChoice>({
    joinMode: 'open',
    listed: initiallyListed,
  });
  const [liveView, setLiveView] = useState(true);
  const [project, setProject] = useState(false);
  const [projectReady, setProjectReady] = useState<PrivateRoom | null>(null);
  const forProject = projectRoom !== undefined && listing.listed && project;
  // 房间建好了、实况没开成：不再建，按钮换成“进房间”。
  const [liveViewMissed, setLiveViewMissed] = useState<{
    readonly failure: PrivateRoomFailure;
    readonly room: PrivateRoom;
  } | null>(null);
  // 只有公开、直接进的房间能放实况：要申请、只能邀请的不出这个勾。项目房间建出来就开着实况。
  const offersLiveView =
    onTurnOnLiveView !== undefined && listing.listed && listing.joinMode === 'open' && !forProject;
  const [retention, setRetention] = useState('');
  const [inviteText, setInviteText] = useState('');
  const [invitePermissions, setInvitePermissions] = useState<PrivateRoomPermissions>(
    permissions('view', 'speak'),
  );
  const [pending, setPending] = useState(false);
  const pendingRef = useRef(false);
  const [failure, setFailure] = useState<string | null>(null);
  // 第一次提交后请求就定下来了：结果不明或进房间失败时，重试必须是同一个请求。开不开实况也一起定下来。
  const attempt = useRef<{
    readonly id: string;
    readonly input: CreatePrivateRoomInput;
    readonly liveView: boolean;
  } | null>(null);
  const locked = attempt.current !== null;
  const rejected = failure !== null && REJECTED_INPUT.has(failure);
  const invitations = parseInvitationList(inviteText);
  const inviteError = invitations.ok
    ? undefined
    : t(
        invitations.reason === 'too_many'
          ? 'rooms.create.invitesTooMany'
          : 'rooms.create.invitesInvalid',
      );
  const ready = name.trim().length > 0 && invitations.ok;

  const enter = async (room: PrivateRoom): Promise<void> => {
    await onEnter(room);
    onClose();
  };

  const enterOnce = async (room: PrivateRoom): Promise<void> => {
    if (pendingRef.current) return;
    pendingRef.current = true;
    setPending(true);
    try {
      await enter(room);
    } finally {
      pendingRef.current = false;
      setPending(false);
    }
  };

  const submit = async (): Promise<void> => {
    if (pendingRef.current) return;
    if (liveViewMissed !== null) {
      await enterOnce(liveViewMissed.room);
      return;
    }
    // 项目房间由服务器认上仓库以后建：这里只有建好以后的“进房间”。
    if (forProject) {
      if (projectReady !== null) await enterOnce(projectReady);
      return;
    }
    if (!locked && !ready) return;
    pendingRef.current = true;
    attempt.current ??= {
      liveView: offersLiveView && liveView,
      id: identifiers.next(),
      input: {
        name: name.trim(),
        description: purpose.trim(),
        invitations: invitations.ok
          ? invitations.principalIds.map((principalId) => ({
              permissions: invitePermissions,
              principalId,
            }))
          : [],
        ...(retention === '' ? {} : { retentionDays: Number(retention) }),
        ...(listing.listed
          ? {
              listing: {
                allowNetworkAgents: true,
                joinMode: listing.joinMode,
                listed: true,
                memberPermissions: DEFAULT_INVITE_PERMISSIONS,
              },
            }
          : {}),
      },
    };
    setPending(true);
    setFailure(null);
    try {
      const result = await onCreate(attempt.current.id, attempt.current.input);
      if (result.ok) {
        if (attempt.current.liveView && onTurnOnLiveView !== undefined) {
          const turned = await onTurnOnLiveView(result.value.catalogId);
          if (!turned.ok) {
            // 房间已经建好了：不再建，说一句实况没开、在设置里能开，按钮换成“进房间”。
            setLiveViewMissed({ failure: turned.error, room: result.value });
            return;
          }
        }
        await enter(result.value);
        return;
      }
      if (REJECTED_INPUT.has(result.error.code)) attempt.current = null;
      setFailure(result.error.code);
    } catch {
      setFailure('rooms.create_interrupted');
    } finally {
      pendingRef.current = false;
      setPending(false);
    }
  };

  return (
    <Dialog
      className="new-room"
      closeLabel={t('rooms.close')}
      description={t('rooms.create.detail')}
      footer={
        forProject ? (
          projectReady === null ? undefined : (
            <Button
              disabled={pending}
              form={formId}
              icon={pending ? <Spinner /> : <DoorOpen aria-hidden="true" />}
              type="submit"
            >
              {t('rooms.create.enter')}
            </Button>
          )
        ) : (
          <Button
            disabled={pending || (!locked && !ready)}
            form={formId}
            icon={pending ? <Spinner /> : <DoorOpen aria-hidden="true" />}
            type="submit"
          >
            {t(
              liveViewMissed !== null
                ? 'rooms.create.enter'
                : pending
                  ? 'rooms.create.creating'
                  : failure === null || rejected
                    ? 'rooms.create.submit'
                    : 'rooms.retry',
            )}
          </Button>
        )
      }
      onClose={() => {
        if (!pending) onClose();
      }}
      title={t('rooms.new')}
    >
      <form
        className="new-room__form"
        id={formId}
        onSubmit={(event) => {
          event.preventDefault();
          void submit();
        }}
      >
        {forProject ? null : (
          <>
            <Field label={t('rooms.create.name')}>
              <input
                data-autofocus
                disabled={pending || locked}
                maxLength={128}
                onChange={(event) => {
                  setName(event.target.value);
                }}
                placeholder={t('rooms.create.namePlaceholder')}
                required
                value={name}
              />
            </Field>
            <Field label={t('rooms.create.purpose')}>
              <textarea
                disabled={pending || locked}
                maxLength={2_048}
                onChange={(event) => {
                  setPurpose(event.target.value);
                }}
                placeholder={t('rooms.create.purposePlaceholder')}
                rows={3}
                value={purpose}
              />
            </Field>
          </>
        )}
        <RoomListingChoices
          askJoinMode={!forProject}
          disabled={pending || locked}
          name={formId}
          onChange={(next) => {
            setListing((current) => ({ ...current, ...next }));
          }}
          value={listing}
        />
        {projectRoom !== undefined && listing.listed ? (
          <label className="room-listing__toggle">
            <input
              checked={project}
              disabled={pending || locked}
              onChange={(event) => {
                setProject(event.target.checked);
              }}
              type="checkbox"
            />
            <span>
              <strong>{t('rooms.create.project')}</strong>
              <small>{t('rooms.create.projectDetail')}</small>
            </span>
          </label>
        ) : null}
        {forProject ? (
          <NewProjectRoom onReady={setProjectReady} ready={projectReady} source={projectRoom} />
        ) : listing.listed ? (
          <p className="room-listing__note room-listing__note--plain">
            {t('rooms.create.publicNote')}
          </p>
        ) : null}
        {offersLiveView ? (
          <label className="room-listing__toggle">
            <input
              checked={liveView}
              disabled={pending || locked}
              onChange={(event) => {
                setLiveView(event.target.checked);
              }}
              type="checkbox"
            />
            <span>
              <strong>{t('rooms.create.liveView')}</strong>
              <small>{t('rooms.create.liveViewDetail')}</small>
            </span>
          </label>
        ) : null}
        {forProject ? null : (
          <Details className="new-room__more" summary={t('rooms.create.more')}>
            <Field label={t('rooms.create.retention')}>
              <select
                disabled={pending || locked}
                onChange={(event) => {
                  setRetention(event.target.value);
                }}
                value={retention}
              >
                <option value="">{t('rooms.create.retentionDefault')}</option>
                {RETENTION_CHOICES.map((days) => (
                  <option key={days} value={String(days)}>
                    {t('rooms.create.retentionDays', { count: days })}
                  </option>
                ))}
              </select>
            </Field>
            <Field
              error={inviteError}
              hint={t('rooms.create.invitesHint')}
              label={t('rooms.create.invites')}
            >
              <textarea
                disabled={pending || locked}
                onChange={(event) => {
                  setInviteText(event.target.value);
                }}
                placeholder={t('rooms.create.invitesPlaceholder')}
                rows={3}
                value={inviteText}
              />
            </Field>
            {invitations.ok && invitations.principalIds.length > 0 ? (
              <PrivateRoomCapabilityEditor
                disabled={pending || locked}
                legend={t('rooms.create.permissions')}
                onChange={setInvitePermissions}
                value={invitePermissions}
              />
            ) : null}
          </Details>
        )}
      </form>
      {liveViewMissed === null ? null : (
        <Banner tone="warning">
          <p>{t('rooms.create.liveViewFailed')}</p>
          <Details summary={t('rooms.details')}>
            <code>{liveViewMissed.failure.code}</code>
          </Details>
        </Banner>
      )}
      {failure === 'private_room.public_limit_reached' ? (
        <Banner tone="warning">{t('rooms.create.publicLimit')}</Banner>
      ) : failure === null ? null : (
        <Banner tone="danger" title={t(rejected ? 'rooms.create.rejected' : 'rooms.create.failed')}>
          <Details summary={t('rooms.details')}>
            <code>{failure}</code>
          </Details>
        </Banner>
      )}
    </Dialog>
  );
}
