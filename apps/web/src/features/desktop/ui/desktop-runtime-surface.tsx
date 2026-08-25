import { Button } from '@agent-room/ui-system';
import {
  AlertTriangle,
  Check,
  ChevronDown,
  ExternalLink,
  KeyRound,
  MonitorCog,
  RefreshCw,
  RotateCcw,
  X,
} from 'lucide-react';
import { AnimatePresence, motion, useReducedMotion } from 'motion/react';
import { useMemo, useState } from 'react';
import { useTranslation } from 'react-i18next';

import type { BridgePhase, DesktopRuntimeGateway } from '@/features/desktop/domain/desktop-runtime';
import { useDesktopRuntime } from '@/features/desktop/ui/use-desktop-runtime';
import type { TranslationKey } from '@/shared/i18n/resources';

const phaseMessage: Readonly<Record<BridgePhase, TranslationKey>> = {
  authorization_required: 'desktop.phase.authorizationRequired',
  discovering: 'desktop.phase.discovering',
  halted: 'desktop.phase.halted',
  ready: 'desktop.phase.ready',
  retry_scheduled: 'desktop.phase.retryScheduled',
  starting: 'desktop.phase.starting',
  stopped: 'desktop.phase.stopped',
};

export type DesktopRuntimeSurfaceProps = {
  readonly gateway?: DesktopRuntimeGateway;
};

export function DesktopRuntimeSurface({ gateway }: DesktopRuntimeSurfaceProps) {
  const { i18n, t } = useTranslation();
  const controller = useDesktopRuntime(gateway);
  const reduceMotion = useReducedMotion();
  const [expanded, setExpanded] = useState(false);
  const phase = controller.snapshot?.bridge.lifecycle.phase ?? 'discovering';
  const authorization = controller.snapshot?.bridge.authorization ?? null;
  const needsAttention =
    controller.failure !== null || authorization !== null || phase === 'halted';
  const open = expanded || needsAttention;
  const expiry = useMemo(() => {
    if (authorization === null) {
      return null;
    }
    return new Intl.DateTimeFormat(i18n.resolvedLanguage, {
      hour: '2-digit',
      minute: '2-digit',
    }).format(new Date(authorization.expiresAtUnixMs));
  }, [authorization, i18n.resolvedLanguage]);

  if (!controller.available) {
    return null;
  }

  return (
    <aside
      aria-live={needsAttention ? 'assertive' : 'polite'}
      className="desktop-runtime"
      data-attention={needsAttention ? 'true' : 'false'}
    >
      <button
        aria-expanded={open}
        className="desktop-runtime__trigger"
        onClick={() => setExpanded((current) => !current)}
        type="button"
      >
        <span className="desktop-runtime__mark" data-phase={phase}>
          {phase === 'ready' ? <Check aria-hidden="true" /> : <MonitorCog aria-hidden="true" />}
        </span>
        <span>
          <strong>{t('desktop.runtime.title')}</strong>
          <small>{t(phaseMessage[phase])}</small>
        </span>
        <ChevronDown aria-hidden="true" className="desktop-runtime__chevron" />
      </button>

      <AnimatePresence initial={false}>
        {open ? (
          <motion.div
            animate={{ height: 'auto', opacity: 1 }}
            className="desktop-runtime__panel"
            exit={{ height: 0, opacity: 0 }}
            initial={needsAttention ? false : { height: 0, opacity: 0 }}
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
          </motion.div>
        ) : null}
      </AnimatePresence>
    </aside>
  );
}
