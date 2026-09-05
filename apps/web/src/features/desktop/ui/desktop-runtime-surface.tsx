import { Button } from '@agent-room/ui-system';
import {
  AlertTriangle,
  Check,
  ChevronDown,
  Download,
  ExternalLink,
  KeyRound,
  MonitorCog,
  PlugZap,
  RefreshCw,
  RotateCcw,
  ShieldCheck,
  X,
} from 'lucide-react';
import { AnimatePresence, motion, useReducedMotion } from 'motion/react';
import { useEffect, useMemo, useRef, useState } from 'react';
import { useTranslation } from 'react-i18next';

import type { BridgePhase, ReleaseUpdateChannel } from '@/features/desktop/domain/desktop-runtime';
import { desktopPhaseMessage } from '@/features/desktop/domain/desktop-connection';
import { ManualHostConfiguration } from '@/features/desktop/ui/manual-host-configuration';
import { useDesktopRuntimeController } from '@/features/desktop/ui/desktop-runtime-provider';
import type { FrontendTelemetryGateway } from '@/features/telemetry/domain/frontend-metric';

export type DesktopRuntimeSurfaceProps = {
  readonly placement?: 'action-rail-safe' | 'viewport';
  readonly telemetry?: FrontendTelemetryGateway;
};

export function DesktopRuntimeSurface({
  placement = 'viewport',
  telemetry,
}: DesktopRuntimeSurfaceProps) {
  const { i18n, t } = useTranslation();
  const controller = useDesktopRuntimeController();
  const reduceMotion = useReducedMotion();
  const [expanded, setExpanded] = useState(false);
  const [updateChannel, setUpdateChannel] = useState<ReleaseUpdateChannel>('stable');
  const phase = controller.snapshot?.bridge.lifecycle.phase ?? 'discovering';
  const previousPhase = useRef<BridgePhase | null>(null);
  const reconnectStartedAt = useRef<number | null>(null);
  const authorization = controller.snapshot?.bridge.authorization ?? null;
  const needsAttention =
    controller.failure !== null || authorization !== null || phase === 'halted';
  const open = expanded;
  const selectedUpdate = controller.update?.channel === updateChannel ? controller.update : null;
  const expiry = useMemo(() => {
    if (authorization === null) {
      return null;
    }
    return new Intl.DateTimeFormat(i18n.resolvedLanguage, {
      hour: '2-digit',
      minute: '2-digit',
    }).format(new Date(authorization.expiresAtUnixMs));
  }, [authorization, i18n.resolvedLanguage]);

  useEffect(() => {
    if (!controller.available || telemetry === undefined || previousPhase.current === phase) {
      return;
    }
    const now = performance.now();
    if (phase === 'retry_scheduled' || phase === 'starting') {
      reconnectStartedAt.current ??= now;
    }
    if (phase === 'ready' && reconnectStartedAt.current !== null) {
      void telemetry.record({
        metric: 'bridge_reconnect',
        surface: 'desktop',
        value: now - reconnectStartedAt.current,
      });
      reconnectStartedAt.current = null;
    }
    void telemetry.record({
      metric: 'bridge_availability',
      surface: 'desktop',
      value: phase === 'ready' ? 1 : 0,
    });
    previousPhase.current = phase;
  }, [controller.available, phase, telemetry]);

  if (!controller.available) {
    return null;
  }

  return (
    <aside
      aria-live={needsAttention ? 'assertive' : 'polite'}
      className="desktop-runtime"
      data-attention={needsAttention ? 'true' : 'false'}
      data-placement={placement}
    >
      <button
        aria-expanded={open}
        className="desktop-runtime__trigger"
        onClick={() => {
          setExpanded((current) => !current);
        }}
        type="button"
      >
        <span className="desktop-runtime__mark" data-phase={phase}>
          {phase === 'ready' ? <Check aria-hidden="true" /> : <MonitorCog aria-hidden="true" />}
        </span>
        <span>
          <strong>{t('desktop.runtime.title')}</strong>
          <small>{t(desktopPhaseMessage[phase])}</small>
        </span>
        <ChevronDown aria-hidden="true" className="desktop-runtime__chevron" />
      </button>

      <AnimatePresence initial={false}>
        {open ? (
          <motion.div
            animate={{ height: 'auto', opacity: 1 }}
            className="desktop-runtime__panel"
            exit={{ height: 0, opacity: 0 }}
            initial={{ height: 0, opacity: 0 }}
            transition={
              reduceMotion ? { duration: 0 } : { bounce: 0.08, duration: 0.32, type: 'spring' }
            }
          >
            {authorization === null ? null : (
              <section className="desktop-runtime__authorization">
                <KeyRound aria-hidden="true" />
                <div>
                  <h2>{t('desktop.authorization.title')}</h2>
                  <p>{t('desktop.authorization.description')}</p>
                  <dl>
                    <div>
                      <dt>{t('desktop.authorization.host')}</dt>
                      <dd>{authorization.verificationHost}</dd>
                    </div>
                    <div>
                      <dt>{t('desktop.authorization.code')}</dt>
                      <dd>{authorization.userCode}</dd>
                    </div>
                  </dl>
                  <p className="desktop-runtime__expiry">
                    {t('desktop.authorization.expires', { time: expiry })}
                  </p>
                  <Button
                    disabled={controller.busy !== null}
                    icon={<ExternalLink aria-hidden="true" />}
                    onClick={() => void controller.openAuthorization(authorization.promptId)}
                    size="compact"
                    tone="network"
                  >
                    {t('desktop.authorization.open')}
                  </Button>
                </div>
              </section>
            )}

            {phase === 'halted' ? (
              <section className="desktop-runtime__failure">
                <AlertTriangle aria-hidden="true" />
                <div>
                  <h2>{t('desktop.halted.title')}</h2>
                  <p>{t('desktop.halted.description')}</p>
                  <code>{controller.snapshot?.bridge.lifecycle.diagnosticCode}</code>
                  {controller.snapshot?.bridge.lifecycle.lastFailureCode === null ||
                  controller.snapshot?.bridge.lifecycle.lastFailureCode ===
                    controller.snapshot?.bridge.lifecycle.diagnosticCode ? null : (
                    <p className="desktop-runtime__diagnostic-detail">
                      <span>{t('desktop.halted.lastFailure')}</span>
                      <code>{controller.snapshot?.bridge.lifecycle.lastFailureCode}</code>
                    </p>
                  )}
                  {controller.snapshot?.bridge.lifecycle.lastExitCode === null ? null : (
                    <p className="desktop-runtime__diagnostic-detail">
                      <span>{t('desktop.halted.exitCode')}</span>
                      <code>{controller.snapshot?.bridge.lifecycle.lastExitCode}</code>
                    </p>
                  )}
                  <Button
                    disabled={controller.busy !== null}
                    icon={<RotateCcw aria-hidden="true" />}
                    onClick={() => void controller.retryBridge()}
                    size="compact"
                    tone="alert"
                  >
                    {t('desktop.halted.retry')}
                  </Button>
                </div>
              </section>
            ) : null}

            {controller.failure === null ? null : (
              <section className="desktop-runtime__command-failure">
                <AlertTriangle aria-hidden="true" />
                <div>
                  <p>{t('desktop.failure.description')}</p>
                  <code>{controller.failure.code}</code>
                </div>
                {controller.failure.retryable ? (
                  <Button
                    aria-label={t('desktop.failure.refresh')}
                    disabled={controller.busy !== null}
                    icon={<RefreshCw aria-hidden="true" />}
                    onClick={() => void controller.refresh()}
                    size="compact"
                    tone="quiet"
                  >
                    {t('desktop.failure.refresh')}
                  </Button>
                ) : (
                  <Button
                    aria-label={t('desktop.failure.dismiss')}
                    icon={<X aria-hidden="true" />}
                    onClick={controller.dismissFailure}
                    size="compact"
                    tone="quiet"
                  >
                    {t('desktop.failure.dismiss')}
                  </Button>
                )}
              </section>
            )}

            {controller.snapshot !== null && authorization === null && phase !== 'halted' ? (
              <section className="desktop-runtime__settings">
                <div>
                  <h2>{t('desktop.autostart.title')}</h2>
                  <p>
                    {t('desktop.autostart.description', {
                      platform: t(`desktop.platform.${controller.snapshot.platform}`),
                    })}
                  </p>
                </div>
                <button
                  aria-pressed={controller.snapshot.autostartEnabled}
                  className="desktop-runtime__switch"
                  disabled={controller.busy !== null}
                  onClick={() =>
                    void controller.setAutostart(!controller.snapshot?.autostartEnabled)
                  }
                  type="button"
                >
                  <span aria-hidden="true" />
                  {controller.snapshot.autostartEnabled
                    ? t('desktop.autostart.on')
                    : t('desktop.autostart.off')}
                </button>
              </section>
            ) : null}

            {controller.snapshot !== null && authorization === null ? (
              <section className="desktop-runtime__hosts">
                <div>
                  <h2>{t('desktop.hosts.title')}</h2>
                  <p>{t('desktop.hosts.description')}</p>
                </div>
                <div className="desktop-runtime__host-list">
                  {controller.hosts
                    .filter((host) => host.installed)
                    .map((host) => (
                      <Button
                        disabled={controller.busy !== null || !host.configurable}
                        icon={<PlugZap aria-hidden="true" />}
                        key={host.host}
                        onClick={() => void controller.configureHost(host.host)}
                        size="compact"
                        tone="quiet"
                      >
                        {t(`desktop.hosts.${host.host}`)}
                      </Button>
                    ))}
                  <ManualHostConfiguration
                    configuration={controller.snapshot.manualHostConfiguration}
                  />
                </div>
              </section>
            ) : null}

            {controller.snapshot?.updatesConfigured === true &&
            authorization === null &&
            phase !== 'halted' ? (
              <section className="desktop-runtime__updates">
                <ShieldCheck aria-hidden="true" />
                <div className="desktop-runtime__update-copy">
                  <h2>{t('desktop.update.title')}</h2>
                  <p>{t('desktop.update.description')}</p>
                  <div
                    aria-label={t('desktop.update.channel')}
                    className="desktop-runtime__channels"
                    role="group"
                  >
                    {(['stable', 'testing'] as const).map((channel) => (
                      <button
                        aria-pressed={updateChannel === channel}
                        disabled={controller.busy !== null}
                        key={channel}
                        onClick={() => {
                          setUpdateChannel(channel);
                        }}
                        type="button"
                      >
                        {t(`desktop.update.channel.${channel}`)}
                      </button>
                    ))}
                  </div>
                  {selectedUpdate === null ? null : (
                    <p className="desktop-runtime__update-version">
                      {selectedUpdate.available
                        ? t('desktop.update.available', {
                            current: selectedUpdate.currentVersion,
                            target: selectedUpdate.targetVersion,
                          })
                        : t('desktop.update.current', {
                            version: selectedUpdate.currentVersion,
                          })}
                    </p>
                  )}
                </div>
                <Button
                  disabled={controller.busy !== null}
                  icon={
                    selectedUpdate?.available === true ? (
                      <Download aria-hidden="true" />
                    ) : (
                      <RefreshCw aria-hidden="true" />
                    )
                  }
                  onClick={() =>
                    selectedUpdate?.available === true
                      ? void controller.installUpdate()
                      : void controller.checkUpdate(updateChannel)
                  }
                  size="compact"
                  tone={selectedUpdate?.rollback === true ? 'alert' : 'network'}
                >
                  {selectedUpdate?.available === true
                    ? selectedUpdate.rollback
                      ? t('desktop.update.rollback')
                      : t('desktop.update.install')
                    : t('desktop.update.check')}
                </Button>
              </section>
            ) : null}
          </motion.div>
        ) : null}
      </AnimatePresence>
    </aside>
  );
}
