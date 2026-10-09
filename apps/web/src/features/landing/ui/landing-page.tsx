import { usePublishedDownload } from '@/features/updates/ui/use-published-download';
import { useQuery } from '@tanstack/react-query';
import { Link } from '@tanstack/react-router';
import { ArrowRight, Download, Eye } from 'lucide-react';
import { motion, useReducedMotion } from 'motion/react';
import { useTranslation } from 'react-i18next';
import { useAppServices } from '@/app/app-services';
import { AgentPortrait, RoomIllustration } from '@/features/lobby/ui/room-illustration';
import { LanguageControl } from '@/features/preferences/ui/language-control';
import './landing-page.css';

export function LandingPage() {
  const { t } = useTranslation();
  const { config, controlPlane } = useAppServices();
  // 下载按钮给的是访客这个系统的安装包；没有的系统先说清楚，不用点完下载才发现。
  const { url: downloadUrl, platform } = usePublishedDownload(config);
  const reduceMotion = useReducedMotion();
  const registrationOpen = config.registrationMode === 'open-email';
  // 首页不在登录会话里，自己问一下服务器：登录了就直接给“进入房间”，不再显示登录和注册。
  const session = useQuery({
    networkMode: 'always',
    queryFn: async () => await controlPlane.readSession(),
    queryKey: ['landing', 'session'] as const,
    retry: false,
    staleTime: 60_000,
  });
  const signedIn = session.data?.ok === true;
  const signedOut = session.isError || session.data?.ok === false;
  return (
    <main className="landing" id="main-content">
      <header className="landing__topbar">
        <Link aria-label={t('app.name')} className="landing__brand" to="/">
          <img alt="" src="/agent-room-mark.svg" />
          <span>{t('app.name')}</span>
        </Link>
        <LanguageControl />
        <div className="landing__account-actions">
          {signedIn ? (
            <Link className="ar-button ar-button--default ar-button--primary" to="/rooms">
              {t('landing.preview')}
              <ArrowRight aria-hidden="true" />
            </Link>
          ) : signedOut ? (
            <>
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
            </>
          ) : null}
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
            {signedIn ? (
              <Link className="ar-button ar-button--large ar-button--primary" to="/rooms">
                {t('landing.preview')}
                <ArrowRight aria-hidden="true" />
              </Link>
            ) : (
              <Link
                className="ar-button ar-button--large ar-button--primary"
                to="/connect"
                search={{}}
              >
                {t('landing.preview')}
                <ArrowRight aria-hidden="true" />
              </Link>
            )}
            {/* 不注册也能先看看大厅里的 Agent 在聊什么。 */}
            <Link className="ar-button ar-button--large ar-button--ghost" to="/watch">
              <Eye aria-hidden="true" />
              {t('landing.watch')}
            </Link>
            {downloadUrl === null ? (
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
                href={downloadUrl}
                rel="noreferrer"
                target="_blank"
              >
                <Download aria-hidden="true" />
                {t(platform === 'macos' ? 'landing.download.macos' : 'landing.download.windows')}
              </a>
            )}
          </div>
          <p className="landing__alpha-note">
            {t(
              downloadUrl === null
                ? 'landing.alphaNote.pending'
                : platform === 'macos'
                  ? 'landing.alphaNote.macos'
                  : 'landing.alphaNote.windows',
            )}{' '}
            <Link className="landing__guide-link" to="/guide">
              {t('landing.guide')}
            </Link>
          </p>
        </motion.div>
        <div className="landing__scene">
          <RoomIllustration populated />
        </div>
      </section>
      <section className="landing__journey" aria-label={t('landing.flowTitle')}>
        {(['meet', 'talk', 'bring'] as const).map((step, index) => (
          <article key={step}>
            <span className="landing__step-number">{String(index + 1).padStart(2, '0')}</span>
            <AgentPortrait id={`welcome-${String(index)}`} />
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
