import { prepareForUpdate } from '@/features/updates/application/update-readiness';
import { useNavigate } from '@tanstack/react-router';
import { useCallback, useEffect, useRef, useState } from 'react';
import { useTranslation } from 'react-i18next';

import { TauriDesktopRuntimeGateway } from '@/features/desktop/adapters/tauri-desktop-runtime-gateway';
import { err, type Result } from '@/shared/result';
import { newerUpdateStatus, updateStatusAfterCheck } from '@/features/desktop/domain/update-status';
import {
  parseLobbyDeepLinkRoute,
  type BridgeRuntime,
  type InvitationOffer,
  type PendingInvitation,
  type DesktopAgentTarget,
  type DesktopDeepLink,
  type DesktopNotification,
  type DesktopRuntimeFailure,
  type DesktopRuntimeGateway,
  type DesktopRuntimeSnapshot,
  type HostSessionDiagnostics,
  type ReleaseUpdateChannel,
  type ReleaseUpdateCheck,
  type ReleaseUpdateProgress,
  type ReleaseUpdateStatus,
} from '@/features/desktop/domain/desktop-runtime';

const defaultGateway = new TauriDesktopRuntimeGateway();

type DesktopOperation =
  | 'authorization'
  | 'autostart'
  | 'agent-runtime'
  | 'reauthorize'
  | 'refresh'
  | 'retry'
  | 'update-check'
  | 'update-install';

export type DesktopRuntimeController = {
  readonly available: boolean;
  readonly receptionAvailable: boolean;
  readonly busy: DesktopOperation | null;
  readonly failure: DesktopRuntimeFailure | null;
  readonly snapshot: DesktopRuntimeSnapshot | null;
  /** 最近一次查成的结果，`updateStatus.check` 的简写。 */
  readonly update: ReleaseUpdateCheck | null;
  /** 原生层上次检查更新的结果，自动的、手动的都算；还没查过时为 null。 */
  readonly updateStatus?: ReleaseUpdateStatus | null;
  readonly updateBusy?: 'checking' | 'installing' | null;
  readonly updateProgress?: ReleaseUpdateProgress | null;
  /** 上次安装没成的原因。检查没成的原因在 `updateStatus.failure`。 */
  readonly updateFailure?: DesktopRuntimeFailure | null;
  readonly readHostSessions: () => Promise<
    Result<readonly HostSessionDiagnostics[], DesktopRuntimeFailure>
  >;
  readonly offerInvitation: (
    invitation: InvitationOffer,
  ) => Promise<Result<PendingInvitation | null, DesktopRuntimeFailure>>;
  readonly withdrawInvitation: (sessionKey: string) => Promise<void>;
  readonly checkUpdate: (channel: ReleaseUpdateChannel) => Promise<void>;
  readonly dismissFailure: () => void;
  readonly openAuthorization: (promptId: string) => Promise<void>;
  readonly openLogs: () => Promise<void>;
  readonly notify: (notification: DesktopNotification) => Promise<void>;
  readonly refresh: () => Promise<void>;
  readonly retryBridge: () => Promise<void>;
  readonly reauthorizeBridge: () => Promise<void>;
  readonly installUpdate: () => Promise<void>;
  readonly setAutostart: (enabled: boolean) => Promise<void>;
  readonly bootstrapDefaultAgent: (preferredLanguage: string | null) => Promise<void>;
  readonly configureAgentRuntime: (target: DesktopAgentTarget) => Promise<void>;
};

export function useDesktopRuntime(
  gateway: DesktopRuntimeGateway = defaultGateway,
): DesktopRuntimeController {
  const navigate = useNavigate();
  const available = gateway.isAvailable();
  const { i18n } = useTranslation();
  const [snapshot, setSnapshot] = useState<DesktopRuntimeSnapshot | null>(null);
  const [failure, setFailure] = useState<DesktopRuntimeFailure | null>(null);
  useEffect(() => {
    if (!available || !gateway.setLanguage) return;
    let active = true;
    void gateway
      .setLanguage(i18n.resolvedLanguage?.startsWith('zh') ? 'zh-CN' : 'en')
      .then((result) => {
        if (active && !result.ok) setFailure(result.error);
      })
      .catch(() => {
        if (active) setFailure({ code: 'desktop.language.failed', retryable: true });
      });
    return () => {
      active = false;
    };
  }, [available, gateway, i18n.resolvedLanguage]);
  const [busy, setBusy] = useState<DesktopOperation | null>(null);
  const [updateStatus, setUpdateStatus] = useState<ReleaseUpdateStatus | null>(null);
  const update = updateStatus?.check ?? null;
  const [updateBusy, setUpdateBusy] = useState<'checking' | 'installing' | null>(null);
  const [updateFailure, setUpdateFailure] = useState<DesktopRuntimeFailure | null>(null);
  const [updateProgress, setUpdateProgress] = useState<ReleaseUpdateProgress | null>(null);
  const updateInFlight = useRef(false);
  // 托盘菜单的请求经事件进来，订阅只建一次，所以经 ref 拿到最新的安装；检查经 ref 读上次的结果。
  const installUpdateRef = useRef<() => Promise<void>>(() => Promise.resolve());
  const updateStatusRef = useRef<ReleaseUpdateStatus | null>(null);
  updateStatusRef.current = updateStatus;
  const readHostSessions = useCallback(
    () =>
      gateway.readHostSessions?.() ??
      Promise.resolve(err({ code: 'desktop.hosts.diagnostics_unavailable', retryable: false })),
    [gateway],
  );
  const offerInvitation = useCallback(
    (invitation: InvitationOffer) =>
      gateway.offerInvitation?.(invitation) ??
      Promise.resolve(err({ code: 'desktop.invitation.unavailable', retryable: false })),
    [gateway],
  );
  const withdrawInvitation = useCallback(
    async (sessionKey: string) => {
      await gateway.withdrawInvitation?.(sessionKey);
    },
    [gateway],
  );

  const applyDeepLink = useCallback(
    (target: DesktopDeepLink): void => {
      const navigation = parseLobbyDeepLinkRoute(target.route);
      if (navigation === null) {
        setFailure({ code: 'desktop.deep_link.invalid', retryable: false });
        return;
      }
      if (navigation.kind === 'catalog') {
        void navigate({
          params: { catalogId: navigation.catalogId },
          to: '/lobby/$catalogId',
        });
        return;
      }
      void navigate({
        params: { catalogId: navigation.catalogId, roomId: navigation.roomId },
        search: {},
        to: '/lobby/$catalogId/instance/$roomId',
      });
    },
    [navigate],
  );

  const refresh = useCallback(async (): Promise<void> => {
    if (!available) {
      return;
    }
    setBusy('refresh');
    const result = await gateway.snapshot();
    if (result.ok) {
      setSnapshot(result.value);
      setUpdateStatus((previous) => newerUpdateStatus(previous, result.value.updateStatus ?? null));
      setFailure(null);
      if (result.value.deepLink !== null) {
        applyDeepLink(result.value.deepLink);
      }
    } else {
      setFailure(result.error);
    }
    setBusy(null);
  }, [applyDeepLink, available, gateway]);

  useEffect(() => {
    if (!available) {
      return undefined;
    }
    let disposed = false;
    let unsubscribe: (() => void) | undefined;
    let latestRuntime: BridgeRuntime | undefined;
    void gateway
      .subscribe({
        onDeepLink: (target) => {
          if (!disposed) {
            applyDeepLink(target);
          }
        },
        onFailure: (nextFailure) => {
          if (!disposed) {
            setFailure(nextFailure);
          }
        },
        onRuntimeChanged: (runtime) => {
          latestRuntime = runtime;
          if (!disposed) {
            setSnapshot((previous) =>
              previous === null ? previous : { ...previous, bridge: runtime },
            );
          }
        },
        onUpdateProgress: (progress) => {
          if (!disposed) setUpdateProgress(progress);
        },
        onUpdateStatus: (status) => {
          if (!disposed) setUpdateStatus((previous) => newerUpdateStatus(previous, status));
        },
        // 托盘菜单“更新到 X…”：和点提示上的按钮一样，先看草稿再装。
        onUpdateRequested: () => {
          if (!disposed) void installUpdateRef.current();
        },
      })
      .then((subscription) => {
        if (!subscription.ok) {
          if (!disposed) {
            setFailure(subscription.error);
          }
          return;
        }
        if (disposed) {
          subscription.value();
          return;
        }
        unsubscribe = subscription.value;
      });
    setBusy('refresh');
    void gateway.snapshot().then((result) => {
      if (disposed) {
        return;
      }
      setBusy(null);
      if (!result.ok) {
        setFailure(result.error);
        return;
      }
      setSnapshot({
        ...result.value,
        bridge: latestRuntime ?? result.value.bridge,
      });
      // 更新由原生层定时查（启动 30 秒后第一次，之后每 4 小时），这里只接它上次查的结果。
      setUpdateStatus((previous) => newerUpdateStatus(previous, result.value.updateStatus ?? null));
      if (result.value.deepLink !== null) {
        applyDeepLink(result.value.deepLink);
      }
    });
    return () => {
      disposed = true;
      unsubscribe?.();
    };
  }, [applyDeepLink, available, gateway]);

  const retryBridge = useCallback(async (): Promise<void> => {
    setBusy('retry');
    const result = await gateway.retryBridge();
    if (result.ok) {
      setSnapshot((previous) =>
        previous === null ? previous : { ...previous, bridge: result.value },
      );
      setFailure(null);
    } else {
      setFailure(result.error);
    }
    setBusy(null);
  }, [gateway]);

  const reauthorizeBridge = useCallback(async (): Promise<void> => {
    setBusy('reauthorize');
    const result = await gateway.reauthorizeBridge();
    if (result.ok) {
      setSnapshot((previous) =>
        previous === null ? previous : { ...previous, bridge: result.value },
      );
      setFailure(null);
    } else {
      setFailure(result.error);
    }
    setBusy(null);
  }, [gateway]);

  const setAutostart = useCallback(
    async (enabled: boolean): Promise<void> => {
      setBusy('autostart');
      const result = await gateway.setAutostart(enabled);
      if (result.ok) {
        setSnapshot((previous) =>
          previous === null ? previous : { ...previous, autostartEnabled: result.value },
        );
        setFailure(null);
      } else {
        setFailure(result.error);
      }
      setBusy(null);
    },
    [gateway],
  );

  const openAuthorization = useCallback(
    async (promptId: string): Promise<void> => {
      setBusy('authorization');
      const result = await gateway.openAuthorization(promptId);
      setFailure(result.ok ? null : result.error);
      setBusy(null);
    },
    [gateway],
  );

  const openLogs = useCallback(async (): Promise<void> => {
    const result =
      (await gateway.openLogs?.()) ?? err({ code: 'desktop.logs.unavailable', retryable: false });
    if (!result.ok) setFailure(result.error);
  }, [gateway]);
  // A notification that cannot be shown is not worth an error banner; the in-app notice remains.
  const notify = useCallback(
    async (notification: DesktopNotification): Promise<void> => {
      await gateway.notify?.(notification).catch(() => undefined);
    },
    [gateway],
  );

  // 检查和安装的结果只在更新的提示和“设置 → 这台电脑”里说，不进这台电脑连接的报错。
  const checkUpdate = useCallback(
    async (channel: ReleaseUpdateChannel): Promise<void> => {
      if (updateInFlight.current) return;
      updateInFlight.current = true;
      setUpdateBusy('checking');
      setBusy('update-check');
      setUpdateFailure(null);
      try {
        const result = await gateway
          .checkUpdate(channel)
          .catch(() => err({ code: 'desktop.update.check_failed', retryable: true }));
        const checked = updateStatusAfterCheck(
          updateStatusRef.current,
          channel,
          result,
          Date.now(),
        );
        setUpdateStatus((previous) => newerUpdateStatus(previous, checked));
      } finally {
        updateInFlight.current = false;
        setUpdateBusy(null);
        setBusy(null);
      }
    },
    [gateway],
  );

  const installUpdate = useCallback(async (): Promise<void> => {
    if (!update?.available || updateInFlight.current) return;
    if (!prepareForUpdate()) {
      setUpdateFailure({ code: 'desktop.update.draft_unsaved', retryable: true });
      return;
    }
    updateInFlight.current = true;
    setUpdateProgress(null);
    setUpdateBusy('installing');
    setBusy('update-install');
    setUpdateFailure(null);
    try {
      const result = await gateway.installUpdate(update.channel, update.sequence);
      setUpdateFailure(result.ok ? null : result.error);
    } catch {
      setUpdateFailure({ code: 'desktop.update.install_failed', retryable: true });
    } finally {
      updateInFlight.current = false;
      setUpdateBusy(null);
      setBusy(null);
    }
  }, [gateway, update]);
  installUpdateRef.current = installUpdate;

  const configureAgentRuntime = useCallback(
    async (target: DesktopAgentTarget): Promise<void> => {
      setBusy('agent-runtime');
      const result = await gateway.configureAgentRuntime(target);
      if (result.ok) {
        setSnapshot((previous) =>
          previous === null ? previous : { ...previous, agentTarget: result.value },
        );
        setFailure(null);
      } else {
        setFailure(result.error);
      }
      setBusy(null);
    },
    [gateway],
  );

  const bootstrapDefaultAgent = useCallback(
    async (preferredLanguage: string | null): Promise<void> => {
      setBusy('agent-runtime');
      const result = await gateway.bootstrapDefaultAgent(preferredLanguage);
      if (result.ok) {
        setSnapshot((previous) =>
          previous === null ? previous : { ...previous, agentTarget: result.value },
        );
        setFailure(null);
      } else {
        setFailure(result.error);
      }
      setBusy(null);
    },
    [gateway],
  );

  return {
    available,
    receptionAvailable: gateway.listReceivers !== undefined,
    busy,
    failure,
    snapshot,
    update,
    updateStatus,
    updateBusy,
    updateProgress,
    updateFailure,
    readHostSessions,
    offerInvitation,
    withdrawInvitation,
    checkUpdate,
    bootstrapDefaultAgent,
    configureAgentRuntime,
    dismissFailure: () => {
      setFailure(null);
    },
    openAuthorization,
    openLogs,
    notify,
    refresh,
    retryBridge,
    reauthorizeBridge,
    installUpdate,
    setAutostart,
  };
}
