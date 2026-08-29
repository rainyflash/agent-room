import { Button } from '@agent-room/ui-system';
import { Check, ChevronDown, Code2, Copy, TerminalSquare } from 'lucide-react';
import { AnimatePresence, motion, useReducedMotion } from 'motion/react';
import { useMemo, useState } from 'react';
import { useTranslation } from 'react-i18next';

import { serializeManualHostConfiguration } from '@/features/desktop/domain/manual-host-configuration';
import type { ManualHostConfiguration as ManualHostConfigurationValue } from '@/features/desktop/domain/desktop-runtime';

type CopyState = 'idle' | 'copied' | 'failed';

export function ManualHostConfiguration({
  configuration,
}: {
  readonly configuration: ManualHostConfigurationValue;
}) {
  const { t } = useTranslation();
  const reduceMotion = useReducedMotion();
  const [open, setOpen] = useState(false);
  const [copyState, setCopyState] = useState<CopyState>('idle');
  const serialized = useMemo(
    () => serializeManualHostConfiguration(configuration),
    [configuration],
  );

  const copyConfiguration = async (): Promise<void> => {
    try {
      await navigator.clipboard.writeText(serialized);
      setCopyState('copied');
    } catch {
      setCopyState('failed');
    }
  };

  return (
    <div className="manual-host-config">
      <Button
        aria-expanded={open}
        icon={<Code2 aria-hidden="true" />}
        onClick={() => setOpen((current) => !current)}
        size="compact"
        tone="quiet"
      >
        {t('desktop.hosts.manual.action')}
        <ChevronDown aria-hidden="true" className="manual-host-config__chevron" />
      </Button>
      <AnimatePresence initial={false}>
        {open ? (
          <motion.div
            animate={{ height: 'auto', opacity: 1 }}
            className="manual-host-config__content"
            exit={{ height: 0, opacity: 0 }}
            initial={{ height: 0, opacity: 0 }}
            transition={
              reduceMotion ? { duration: 0 } : { bounce: 0.08, duration: 0.3, type: 'spring' }
            }
          >
            <header>
              <TerminalSquare aria-hidden="true" />
              <div>
                <h3>{t('desktop.hosts.manual.title')}</h3>
                <p>{t('desktop.hosts.manual.description')}</p>
              </div>
            </header>
            <dl>
              <div>
                <dt>{t('desktop.hosts.manual.serverName')}</dt>
                <dd>{configuration.serverName}</dd>
              </div>
              <div>
                <dt>{t('desktop.hosts.manual.transport')}</dt>
                <dd>{configuration.transport.toUpperCase()}</dd>
              </div>
              <div className="manual-host-config__command">
                <dt>{t('desktop.hosts.manual.command')}</dt>
                <dd>{configuration.command}</dd>
              </div>
              <div>
                <dt>{t('desktop.hosts.manual.arguments')}</dt>
                <dd>{configuration.args.length === 0 ? '[]' : configuration.args.join(' ')}</dd>
              </div>
            </dl>
            <div className="manual-host-config__example">
              <div>
                <span>{t('desktop.hosts.manual.example')}</span>
                <Button
                  icon={
                    copyState === 'copied' ? (
                      <Check aria-hidden="true" />
                    ) : (
                      <Copy aria-hidden="true" />
                    )
                  }
                  onClick={() => void copyConfiguration()}
                  size="compact"
                  tone={copyState === 'failed' ? 'alert' : 'quiet'}
                >
                  {t(`desktop.hosts.manual.copy.${copyState}`)}
                </Button>
              </div>
              <pre>
                <code>{serialized}</code>
              </pre>
            </div>
            <p className="manual-host-config__note">{t('desktop.hosts.manual.note')}</p>
          </motion.div>
        ) : null}
      </AnimatePresence>
    </div>
  );
}
