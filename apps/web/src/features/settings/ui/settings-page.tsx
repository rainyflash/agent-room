import { Link } from '@tanstack/react-router';
import { Info, Monitor, ShieldCheck, SlidersHorizontal } from 'lucide-react';
import type { ReactNode } from 'react';
import { useTranslation } from 'react-i18next';

import { ThisComputerSettings } from '@/features/desktop/ui/this-computer-settings';
import { useOptionalDesktopRuntimeController } from '@/features/desktop/ui/desktop-runtime-provider';
import { ApplicationAbout } from '@/features/updates/ui/application-about';
import { AppNavigation } from '@/shared/ui/app-navigation';
import { GeneralSettings } from './general-settings';
import { settingsSections, type SettingsSection } from './settings-sections';
import './settings-page.css';

const sectionPresentation = {
  general: { icon: SlidersHorizontal, label: 'settings.section.general' },
  security: { icon: ShieldCheck, label: 'settings.section.security' },
  'this-computer': { icon: Monitor, label: 'settings.section.thisComputer' },
  about: { icon: Info, label: 'settings.section.about' },
} as const;

/**
 * 设置只有一个家：通用、安全、这台电脑（只在桌面端）、关于。左边（手机上是上方）选分节，
 * 右边是这一节的内容。
 */
export function SettingsLayout({
  section,
  children,
}: {
  readonly section: SettingsSection;
  readonly children: ReactNode;
}) {
  const { t } = useTranslation();
  const desktop = useOptionalDesktopRuntimeController();
  const native = desktop?.available === true;
  const sections = settingsSections.filter((id) => id !== 'this-computer' || native);
  const updateReady = native && desktop.update?.available === true;
  return (
    <main className="settings-page" id="main-content">
      <AppNavigation active="settings" />
      <div className="settings-layout">
        <h1 className="settings-title">{t('settings.title')}</h1>
        <nav aria-label={t('settings.sections')} className="settings-nav">
          {sections.map((id) => {
            const Icon = sectionPresentation[id].icon;
            return (
              <Link
                aria-current={id === section ? 'page' : undefined}
                key={id}
                params={{ section: id }}
                to="/settings/$section"
              >
                <Icon aria-hidden="true" />
                <span>{t(sectionPresentation[id].label)}</span>
                {id === 'this-computer' && updateReady ? (
                  <span className="app-navigation__dot">
                    <span className="sr-only">{t('settings.updateDot')}</span>
                  </span>
                ) : null}
              </Link>
            );
          })}
        </nav>
        <section aria-labelledby="settings-section-title" className="settings-content">
          <h2 id="settings-section-title">{t(sectionPresentation[section].label)}</h2>
          {children}
        </section>
      </div>
    </main>
  );
}

/** 通用、这台电脑、关于的内容；安全一节由路由另外装上（要先连上 Matrix）。 */
export function SettingsSectionContent({
  section,
}: {
  readonly section: Exclude<SettingsSection, 'security'>;
}) {
  switch (section) {
    case 'general':
      return <GeneralSettings />;
    case 'this-computer':
      return <ThisComputerSettings />;
    case 'about':
      return <ApplicationAbout />;
  }
}
