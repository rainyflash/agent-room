import { Link } from '@tanstack/react-router';
import { ArrowLeft } from 'lucide-react';
import { useTranslation } from 'react-i18next';

import { detectVisitorPlatform } from '@/features/landing/domain/visitor-platform';
import { LanguageControl } from '@/features/preferences/ui/language-control';
import './guide-page.css';

const REPOSITORY_URL = 'https://github.com/rainyflash/agent-room';
const steps = ['account', 'install', 'invite', 'reply'] as const;
const questions = ['install', 'sees', 'control', 'hosts', 'alpha'] as const;
const links = [
  { key: 'repository', href: REPOSITORY_URL },
  { key: 'selfHosting', href: `${REPOSITORY_URL}/blob/main/docs/self-hosting.md` },
  { key: 'security', href: `${REPOSITORY_URL}/blob/main/SECURITY.md` },
] as const;

export function GuidePage() {
  const { t } = useTranslation();
  // 装不了桌面端的访客先知道这一点，不必读完整段再发现自己的系统没有安装包。
  const platform = detectVisitorPlatform(window.navigator.userAgent);
  return (
    <main className="guide" id="main-content">
      <header className="guide__topbar">
        <Link aria-label={t('app.name')} className="guide__brand" to="/">
          <img alt="" src="/agent-room-mark.svg" />
          <span>{t('app.name')}</span>
        </Link>
        <LanguageControl />
        <Link className="ar-button ar-button--default ar-button--ghost" to="/">
          <ArrowLeft aria-hidden="true" />
          {t('guide.back')}
        </Link>
      </header>
      <section className="guide__intro">
        <h1>{t('guide.title')}</h1>
        <p className="guide__lede">{t('guide.lede')}</p>
      </section>
      <ol className="guide__steps">
        {steps.map((step, index) => (
          <li key={step}>
            <span className="guide__step-label">{t('guide.stepLabel', { number: index + 1 })}</span>
            <h2>{t(`guide.step.${step}.title`)}</h2>
            <p>{t(`guide.step.${step}.detail`)}</p>
            {step === 'install' ? (
              <p className="guide__platform-note">{t(`guide.step.install.platform.${platform}`)}</p>
            ) : null}
          </li>
        ))}
      </ol>
      <section className="guide__faq" aria-labelledby="guide-faq-title">
        <h2 id="guide-faq-title">{t('guide.faq.title')}</h2>
        <dl>
          {questions.map((question) => (
            <div key={question}>
              <dt>{t(`guide.faq.${question}.question`)}</dt>
              <dd>{t(`guide.faq.${question}.answer`)}</dd>
            </div>
          ))}
        </dl>
      </section>
      <section className="guide__links" aria-labelledby="guide-links-title">
        <h2 id="guide-links-title">{t('guide.links.title')}</h2>
        <ul>
          {links.map((link) => (
            <li key={link.key}>
              <a href={link.href} rel="noreferrer" target="_blank">
                {t(`guide.links.${link.key}`)}
              </a>
            </li>
          ))}
        </ul>
      </section>
    </main>
  );
}
