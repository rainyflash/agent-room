import { prepareForUpdate } from '@/features/updates/application/update-readiness';
import { useNavigate } from '@tanstack/react-router';
import { useCallback, useEffect, useRef, useState } from 'react';
import { useTranslation } from 'react-i18next';

import { TauriDesktopRuntimeGateway } from '@/features/desktop/adapters/tauri-desktop-runtime-gateway';
import { err, ok, type Result } from '@/shared/result';
import {
  defaultReleaseChannel,
  parseLobbyDeepLinkRoute,
  type BridgeRuntime,
  type AgentHostDetection,
  type AgentHostKind,
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
} from '@/features/desktop/domain/desktop-runtime';

const defaultGateway = new TauriDesktopRuntimeGateway();

type DesktopOperation =
  | 'authorization'
  | 'autostart'
  | 'agent-runtime'
  | 'host-configure'
  | 'reauthorize'
  | 'refresh'
  | 'retry'
  | 'update-check'
  | 'update-install';

export type HostSetupState =
  | { readonly phase: 'checking' | 'required' | 'configured' }
  | { readonly phase: 'failed'; readonly error: DesktopRuntimeFailure };

export type DesktopRuntimeController = {
  readonly available: boolean;
  readonly receptionAvailable: boolean;
  readonly busy: DesktopOperation | null;
  readonly failure: DesktopRuntimeFailure | null;
  readonly snapshot: DesktopRuntimeSnapshot | null;
  readonly update: ReleaseUpdateCheck | null;
  readonly updateBusy?: 'checking' | 'installing' | null;
  readonly updateProgress?: ReleaseUpdateProgress | null;
  readonly updateFailure?: DesktopRuntimeFailure | null;
  readonly hosts: readonly AgentHostDetection[];
  readonly configuredHost: AgentHostKind | null;
  readonly hostSetup: Readonly<Partial<Record<AgentHostKind, HostSetupState>>>;
  readonly checkHost: (host: AgentHostKind) => Promise<void>;
  readonly readHostSessions: () => Promise<
    Result<readonly HostSessionDiagnostics[], DesktopRuntimeFailure>
  >;
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
  readonly configureHost: (host: AgentHostKind) => Promise<void>;
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
  const [update, setUpdate] = useState<ReleaseUpdateCheck | null>(null);
  const [updateBusy, setUpdateBusy] = useState<'checking' | 'installing' | null>(null);
  const [updateFailure, setUpdateFailure] = useState<DesktopRuntimeFailure | null>(null);
  const [updateProgress, setUpdateProgress] = useState<ReleaseUpdateProgress | null>(null);
  const updateInFlight = useRef(false);
  const [hosts, setHosts] = useState<readonly AgentHostDetection[]>([]);
  const [configuredHost, setConfiguredHost] = useState<AgentHostKind | null>(null);
  const [hostSetup, setHostSetup] = useState<Partial<Record<AgentHostKind, HostSetupState>>>({});
  const hostChecks = useRef<Partial<Record<AgentHostKind, number>>>({});
  const configuringHosts = useRef(new Set<AgentHostKind>());
  const checkHost = useCallback(
    async (host: AgentHostKind): Promise<void> => {
      if (configuringHosts.current.has(host)) return;
      const generation = (hostChecks.current[host] ?? 0) + 1;
      hostChecks.current[host] = generation;
      setHostSetup((previous) => ({ ...previous, [host]: { phase: 'checking' } }));
      const plan =
        (await gateway.planHost?.(host)) ??
        err({ code: 'desktop.hosts.configuration_unavailable', retryable: false });
      if (hostChecks.current[host] !== generation) return;
      setHostSetup((previous) => ({
        ...previous,
        [host]: plan.ok
          ? { phase: plan.value.action === 'unchanged' ? 'configured' : 'required' }
          : { phase: 'failed', error: plan.error },
      }));
    },
    [gateway],
  );
  const readHostSessions = useCallback(
    () =>
      gateway.readHostSessions?.() ??
      Promise.resolve(err({ code: 'desktop.hosts.diagnostics_unavailable', retryable: false })),
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
      // Nobody clicks "check for updates" on their own: look once per launch on the channel this
      // build came from, and only surface a result. Failures (offline, no manifest) stay silent here.
      if (result.value.updatesConfigured && result.value.currentVersion !== undefined) {
        void gateway
          .checkUpdate(defaultReleaseChannel(result.value.currentVersion))
          .then((check) => {
            if (!disposed && check.ok) setUpdate(check.value);
          })
          .catch(() => undefined);
      }
      if (gateway.detectHosts !== undefined) {
        void gateway.detectHosts().then((hostsResult) => {
          if (!disposed) {
            if (hostsResult.ok) setHosts(hostsResult.value);
            else setFailure(hostsResult.error);
          }
        });
      }
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

  const checkUpdate = useCallback(
    async (channel: ReleaseUpdateChannel): Promise<void> => {
      if (updateInFlight.current) return;
      updateInFlight.current = true;
      setUpdateBusy('checking');
      setBusy('update-check');
      setUpdateFailure(null);
      try {
        const result = await gateway.checkUpdate(channel);
        setUpdate(result.ok ? result.value : null);
        setUpdateFailure(result.ok ? null : result.error);
        setFailure(result.ok ? null : result.error);
      } catch {
        const error: DesktopRuntimeFailure = {
          code: 'desktop.update.check_failed',
          retryable: true,
        };
        setUpdate(null);
        setUpdateFailure(error);
        setFailure(error);
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
      const error: DesktopRuntimeFailure = {
        code: 'desktop.update.draft_unsaved',
        retryable: true,
      };
      setUpdateFailure(error);
      setFailure(error);
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
      setFailure(result.ok ? null : result.error);
    } catch {
      const error: DesktopRuntimeFailure = {
        code: 'desktop.update.install_failed',
        retryable: true,
      };
      setUpdateFailure(error);
      setFailure(error);
    } finally {
      updateInFlight.current = false;
      setUpdateBusy(null);
      setBusy(null);
    }
  }, [gateway, update]);

  const configureHost = useCallback(
    async (host: AgentHostKind): Promise<void> => {
      if (configuringHosts.current.has(host)) return;
      if (gateway.planHost === undefined || gateway.applyHost === undefined) {
        setFailure({ code: 'desktop.hosts.configuration_unavailable', retryable: false });
        return;
      }
      configuringHosts.current.add(host);
      hostChecks.current[host] = (hostChecks.current[host] ?? 0) + 1;
      setBusy('host-configure');
      setConfiguredHost(null);
      setHostSetup((previous) => ({ ...previous, [host]: { phase: 'checking' } }));
      const plan = await gateway.planHost(host);
      if (!plan.ok) {
        setFailure(plan.error);
        setHostSetup((previous) => ({
          ...previous,
          [host]: { phase: 'failed', error: plan.error },
        }));
        setBusy(null);
        configuringHosts.current.delete(host);
        return;
      }
      const result =
        plan.value.action === 'unchanged'
          ? ok(undefined)
          : await gateway.applyHost(host, plan.value.originalDigest);
      setFailure(result.ok ? null : result.error);
      setHostSetup((previous) => ({
        ...previous,
        [host]: result.ok ? { phase: 'configured' } : { phase: 'failed', error: result.error },
      }));
      if (result.ok) setConfiguredHost(host);
      if (result.ok && gateway.detectHosts !== undefined) {
        const detected = await gateway.detectHosts();
        if (detected.ok) setHosts(detected.value);
        else setFailure(detected.error);
      }
      setBusy(null);
      configuringHosts.current.delete(host);
    },
    [gateway],
  );

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
    updateBusy,
    updateProgress,
    updateFailure,
    hosts,
    configuredHost,
    hostSetup,
    checkHost,
    readHostSessions,
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
    configureHost,
  };
}
