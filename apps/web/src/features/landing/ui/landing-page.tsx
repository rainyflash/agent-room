import { Link } from '@tanstack/react-router';
import { ArrowRight, Download } from 'lucide-react';
import { motion, useReducedMotion } from 'motion/react';
import { useTranslation } from 'react-i18next';
import { useAppServices } from '@/app/app-services';
import { AgentPortrait, RoomIllustration } from '@/features/lobby/ui/room-illustration';
import { LanguageControl } from '@/features/preferences/ui/language-control';
import './landing-page.css';

export function LandingPage() {
  const { t } = useTranslation();
  const { config, controlPlane } = useAppServices();
  const reduceMotion = useReducedMotion();
  const registrationOpen = config.registrationMode === 'open-email';
  return (
    <main className="landing" id="main-content">
      <header className="landing__topbar">
        <Link aria-label={t('app.name')} className="landing__brand" to="/">
          <img alt="" src="/agent-room-mark.svg" />
          <span>{t('app.name')}</span>
        </Link>
        <div className="landing__account-actions">
          <LanguageControl />
          <button
            className="ar-button ar-button--default ar-button--ghost"
            onClick={() => {
              void controlPlane.beginAuthentication('/connect', 'sign-in');
            }}
            type="button"
          >
            {t('landing.login')}
          </button>
          <button
            className="ar-button ar-button--default ar-button--primary"
            disabled={!registrationOpen}
            onClick={() => {
              void controlPlane.beginAuthentication('/connect', 'register');
            }}
            type="button"
          >
            {t(registrationOpen ? 'landing.register' : 'landing.registrationPending')}
          </button>
        </div>
      </header>
      <section className="landing__hero">
        <motion.div
          className="landing__copy"
          initial={reduceMotion ? false : { y: 12 }}
          animate={{ y: 0 }}
          transition={{ duration: 0.3 }}
        >
          <h1>{t('landing.title')}</h1>
          <p className="landing__lede">{t('landing.description')}</p>
          <div className="landing__primary-actions">
            <Link
              className="ar-button ar-button--large ar-button--primary"
              to="/connect"
              search={{}}
            >
              {t('landing.preview')}
              <ArrowRight aria-hidden="true" />
            </Link>
            {config.windowsDownloadUrl === null ? (
              <button
                className="ar-button ar-button--large ar-button--ghost"
                disabled
                type="button"
              >
                <Download aria-hidden="true" />
                {t('landing.downloadPending')}
              </button>
            ) : (
              <a
                className="ar-button ar-button--large ar-button--ghost"
                href={config.windowsDownloadUrl}
                rel="noreferrer"
                target="_blank"
              >
                <Download aria-hidden="true" />
                {t('landing.download')}
              </a>
            )}
          </div>
          <p className="landing__alpha-note">{t('landing.alphaNote')}</p>
        </motion.div>
        <div className="landing__scene">
          <RoomIllustration />
        </div>
      </section>
      <section className="landing__journey" aria-label={t('landing.flowTitle')}>
        {(['meet', 'talk', 'bring'] as const).map((step, index) => (
          <article key={step}>
            <span className="landing__step-number">{String(index + 1).padStart(2, '0')}</span>
            <AgentPortrait id={step} />
            <div>
              <h2>{t(`landing.flow.${step}.title`)}</h2>
              <p>{t(`landing.flow.${step}.detail`)}</p>
            </div>
          </article>
        ))}
      </section>
    </main>
  );
}
