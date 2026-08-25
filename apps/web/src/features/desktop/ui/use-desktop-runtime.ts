import { useNavigate } from '@tanstack/react-router';
import { useCallback, useEffect, useState } from 'react';

import { TauriDesktopRuntimeGateway } from '@/features/desktop/adapters/tauri-desktop-runtime-gateway';
import {
  parseLobbyDeepLinkRoute,
  type BridgeRuntime,
  type DesktopDeepLink,
  type DesktopRuntimeFailure,
  type DesktopRuntimeGateway,
  type DesktopRuntimeSnapshot,
} from '@/features/desktop/domain/desktop-runtime';

const defaultGateway = new TauriDesktopRuntimeGateway();

type DesktopOperation = 'authorization' | 'autostart' | 'refresh' | 'retry';

export type DesktopRuntimeController = {
  readonly available: boolean;
  readonly busy: DesktopOperation | null;
  readonly failure: DesktopRuntimeFailure | null;
  readonly snapshot: DesktopRuntimeSnapshot | null;
  readonly dismissFailure: () => void;
  readonly openAuthorization: (promptId: string) => Promise<void>;
  readonly refresh: () => Promise<void>;
  readonly retryBridge: () => Promise<void>;
  readonly setAutostart: (enabled: boolean) => Promise<void>;
};

export function useDesktopRuntime(
  gateway: DesktopRuntimeGateway = defaultGateway,
): DesktopRuntimeController {
  const navigate = useNavigate();
  const available = gateway.isAvailable();
  const [snapshot, setSnapshot] = useState<DesktopRuntimeSnapshot | null>(null);
  const [failure, setFailure] = useState<DesktopRuntimeFailure | null>(null);
  const [busy, setBusy] = useState<DesktopOperation | null>(null);

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

  return {
    available,
    busy,
    failure,
    snapshot,
    dismissFailure: () => {
      setFailure(null);
    },
    openAuthorization,
    refresh,
    retryBridge,
    setAutostart,
  };
}
