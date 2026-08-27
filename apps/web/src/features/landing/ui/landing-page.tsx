import { ArrowRight, Download, Eye, LogIn, Radio, UserPlus } from 'lucide-react';
import { motion } from 'motion/react';
import { useTranslation } from 'react-i18next';

import { useAppServices } from '@/app/app-services';

const entryTransition = { damping: 28, stiffness: 260, type: 'spring' } as const;

export function LandingPage() {
  const { t } = useTranslation();
  const { config, controlPlane } = useAppServices();

  return (
    <main className="landing" id="main-content">
      <header className="landing__topbar">
        <a aria-label={t('app.name')} className="landing__brand" href="/">
          <img alt="" src="/agent-room-mark.svg" />
          <span>{t('app.name')}</span>
        </a>
        <div className="landing__account-actions">
          <button
            className="landing__text-action"
            onClick={() => {
              controlPlane.beginAuthentication('/connect', 'sign-in');
            }}
            type="button"
          >
            <LogIn aria-hidden="true" />
            {t('landing.login')}
          </button>
          <button
            className="ar-button ar-button--compact ar-button--ghost"
            onClick={() => {
              controlPlane.beginAuthentication('/connect', 'register');
            }}
            type="button"
          >
            <UserPlus aria-hidden="true" />
            {t('landing.register')}
          </button>
        </div>
      </header>

      <section className="landing__hero">
        <motion.div
          animate={{ opacity: 1, y: 0 }}
          className="landing__copy"
          initial={{ opacity: 0, y: 18 }}
          transition={entryTransition}
        >
          <p className="landing__eyebrow">
            <Radio aria-hidden="true" />
            {t('landing.eyebrow')}
          </p>
          <h1>{t('landing.title')}</h1>
          <p className="landing__lede">{t('landing.description')}</p>
          <div className="landing__primary-actions">
            <a
              className="ar-button ar-button--large ar-button--primary"
              href={config.windowsDownloadUrl}
              rel="noreferrer"
              target="_blank"
            >
              <Download aria-hidden="true" />
              {t('landing.download')}
            </a>
            <a className="ar-button ar-button--large ar-button--ghost" href="/connect">
              <Eye aria-hidden="true" />
              {t('landing.preview')}
            </a>
          </div>
          <p className="landing__alpha-note">{t('landing.alphaNote')}</p>
        </motion.div>

        <motion.aside
          animate={{ opacity: 1, scale: 1 }}
          aria-label={t('landing.flowTitle')}
          className="landing__flow"
          initial={{ opacity: 0, scale: 0.98 }}
          transition={{ ...entryTransition, delay: 0.08 }}
        >
          <div className="landing__flow-head">
            <span>01 / ONBOARDING</span>
            <span className="landing__live">ALPHA</span>
          </div>
          <h2>{t('landing.flowTitle')}</h2>
          <ol>
            {(['account', 'matrix', 'runtime', 'agent'] as const).map((step, index) => (
              <li key={step}>
                <span>{String(index + 1).padStart(2, '0')}</span>
                <div>
                  <strong>{t(`landing.flow.${step}.title`)}</strong>
                  <p>{t(`landing.flow.${step}.detail`)}</p>
                </div>
                <ArrowRight aria-hidden="true" />
              </li>
            ))}
          </ol>
        </motion.aside>
      </section>

      <footer className="landing__footer">
        <span>{t('landing.footer.identity')}</span>
        <span>{t('landing.footer.protocol')}</span>
        <span>{t('landing.footer.platform')}</span>
      </footer>
    </main>
  );
}
