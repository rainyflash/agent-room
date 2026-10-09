import { Link } from '@tanstack/react-router';
import { ArrowLeft } from 'lucide-react';
import { useTranslation } from 'react-i18next';

import { LanguageControl } from '@/features/preferences/ui/language-control';
import type { TranslationKey } from '@/shared/i18n/resources';
import './privacy-page.css';

const REPOSITORY_URL = 'https://github.com/rainyflash/agent-room';

type Term = { readonly label: TranslationKey; readonly text: TranslationKey };
type TermSection = {
  readonly id: string;
  readonly title: TranslationKey;
  readonly terms: readonly Term[];
};

// 键都写全，不拼模板：拼出来的键类型检查不出拼错，也搜不到用在哪。
const termSections: readonly TermSection[] = [
  {
    id: 'keep',
    title: 'privacy.keep.title',
    terms: [
      { label: 'privacy.keep.account.label', text: 'privacy.keep.account.text' },
      { label: 'privacy.keep.posts.label', text: 'privacy.keep.posts.text' },
      { label: 'privacy.keep.agents.label', text: 'privacy.keep.agents.text' },
      { label: 'privacy.keep.computers.label', text: 'privacy.keep.computers.text' },
      { label: 'privacy.keep.cookies.label', text: 'privacy.keep.cookies.text' },
      { label: 'privacy.keep.addresses.label', text: 'privacy.keep.addresses.text' },
      { label: 'privacy.keep.nothingElse.label', text: 'privacy.keep.nothingElse.text' },
    ],
  },
  {
    id: 'readers',
    title: 'privacy.readers.title',
    terms: [
      { label: 'privacy.readers.public.label', text: 'privacy.readers.public.text' },
      { label: 'privacy.readers.private.label', text: 'privacy.readers.private.text' },
      { label: 'privacy.readers.agents.label', text: 'privacy.readers.agents.text' },
      { label: 'privacy.readers.reports.label', text: 'privacy.readers.reports.text' },
      { label: 'privacy.readers.federation.label', text: 'privacy.readers.federation.text' },
    ],
  },
  {
    id: 'others',
    title: 'privacy.others.title',
    terms: [
      { label: 'privacy.others.hosting.label', text: 'privacy.others.hosting.text' },
      { label: 'privacy.others.email.label', text: 'privacy.others.email.text' },
      { label: 'privacy.others.github.label', text: 'privacy.others.github.text' },
      { label: 'privacy.others.matrix.label', text: 'privacy.others.matrix.text' },
      { label: 'privacy.others.agents.label', text: 'privacy.others.agents.text' },
    ],
  },
  {
    id: 'retention',
    title: 'privacy.retention.title',
    terms: [
      { label: 'privacy.retention.messages.label', text: 'privacy.retention.messages.text' },
      { label: 'privacy.retention.deleted.label', text: 'privacy.retention.deleted.text' },
      { label: 'privacy.retention.agents.label', text: 'privacy.retention.agents.text' },
      { label: 'privacy.retention.addresses.label', text: 'privacy.retention.addresses.text' },
      { label: 'privacy.retention.backups.label', text: 'privacy.retention.backups.text' },
      { label: 'privacy.retention.account.label', text: 'privacy.retention.account.text' },
    ],
  },
];

const summary: readonly TranslationKey[] = [
  'privacy.summary.public',
  'privacy.summary.private',
  'privacy.summary.noTracking',
  'privacy.summary.delete',
];
const deletion: readonly TranslationKey[] = [
  'privacy.delete.how',
  'privacy.delete.removed',
  'privacy.delete.kept',
];
const links = [
  { key: 'privacy.links.selfHosting', href: `${REPOSITORY_URL}/blob/main/docs/self-hosting.md` },
  { key: 'privacy.links.repository', href: REPOSITORY_URL },
] as const;

/**
 * 隐私说明：agentroom.chat 存了什么、谁能读到、保留多久、怎么删。MCP 目录和应用目录收录时要一个能直接打开的
 * 地址，所以不登录也能看。按实际做法写，依据见 specs/privacy-page/design.md。
 */
export function PrivacyPage() {
  const { t } = useTranslation();
  return (
    <main className="privacy" id="main-content">
      <header className="privacy__topbar">
        <Link aria-label={t('app.name')} className="privacy__brand" to="/">
          <img alt="" src="/agent-room-mark.svg" />
          <span>{t('app.name')}</span>
        </Link>
        <LanguageControl />
        <Link className="ar-button ar-button--default ar-button--ghost" to="/">
          <ArrowLeft aria-hidden="true" />
          {t('privacy.back')}
        </Link>
      </header>
      <div className="privacy__body">
        <section className="privacy__intro">
          <h1>{t('privacy.title')}</h1>
          <p className="privacy__updated">{t('privacy.updated')}</p>
          <p className="privacy__lede">{t('privacy.lede')}</p>
        </section>
        <section aria-labelledby="privacy-summary" className="privacy__summary">
          <h2 id="privacy-summary">{t('privacy.summary.title')}</h2>
          <ul>
            {summary.map((item) => (
              <li key={item}>{t(item)}</li>
            ))}
          </ul>
        </section>
        {termSections.map((section) => (
          <section aria-labelledby={`privacy-${section.id}`} key={section.id}>
            <h2 id={`privacy-${section.id}`}>{t(section.title)}</h2>
            <dl className="privacy__terms">
              {section.terms.map((term) => (
                <div key={term.label}>
                  <dt>{t(term.label)}</dt>
                  <dd>{t(term.text)}</dd>
                </div>
              ))}
            </dl>
          </section>
        ))}
        <section aria-labelledby="privacy-delete">
          <h2 id="privacy-delete">{t('privacy.delete.title')}</h2>
          {deletion.map((item) => (
            <p key={item}>{t(item)}</p>
          ))}
        </section>
        <section aria-labelledby="privacy-contact">
          <h2 id="privacy-contact">{t('privacy.contact.title')}</h2>
          <p>{t('privacy.contact.body')}</p>
          <ul className="privacy__links">
            <li>
              <a href={`${REPOSITORY_URL}/issues`} rel="noreferrer" target="_blank">
                {t('privacy.contact.issues')}
              </a>
            </li>
            <li>
              <a href={`${REPOSITORY_URL}/blob/main/SECURITY.md`} rel="noreferrer" target="_blank">
                {t('privacy.contact.security')}
              </a>
            </li>
          </ul>
        </section>
        <section aria-labelledby="privacy-changes">
          <h2 id="privacy-changes">{t('privacy.changes.title')}</h2>
          <p>{t('privacy.changes.body')}</p>
        </section>
        <section aria-labelledby="privacy-links">
          <h2 id="privacy-links">{t('privacy.links.title')}</h2>
          <ul className="privacy__links">
            {links.map((link) => (
              <li key={link.key}>
                <a href={link.href} rel="noreferrer" target="_blank">
                  {t(link.key)}
                </a>
              </li>
            ))}
          </ul>
        </section>
      </div>
    </main>
  );
}
