import { Details } from '@agent-room/ui-system';
import type { ReactNode } from 'react';
import { useTranslation } from 'react-i18next';

import { LanguageControl } from '@/features/preferences/ui/language-control';

/**
 * 还没进到房间里的页面共用的外壳：连接、进房间、找不到页面、配置出错。一条细顶栏放品牌和语言，
 * 下面居中放一张卡片。
 */
export function EntryShell({
  children,
  home = true,
}: {
  readonly children: ReactNode;
  /** 品牌是否链回首页。配置出错时首页也打不开，就不给链接。 */
  readonly home?: boolean;
}) {
  const { t } = useTranslation();
  const brand = (
    <>
      <img alt="" src="/agent-room-mark.svg" />
      <span>{t('app.name')}</span>
    </>
  );
  return (
    <div className="entry-shell">
      <header className="entry-shell__topbar">
        {home ? (
          <a className="brand-lockup" href="/">
            {brand}
          </a>
        ) : (
          <div className="brand-lockup">{brand}</div>
        )}
        <LanguageControl />
      </header>
      {children}
    </div>
  );
}

export type EntryCardProps = {
  readonly title: string;
  /** 标题下面的一两句说明。 */
  readonly detail?: ReactNode;
  readonly children?: ReactNode;
  /** 标题上方的图标：加载时放转圈，出错时放提醒图标。 */
  readonly icon?: ReactNode;
  readonly actions?: ReactNode;
  /** 错误码、路径这类排查才看的信息，收在卡片底部的“详情”里。 */
  readonly details?: ReactNode;
  readonly tone?: 'neutral' | 'alert';
};

/** 一张状态卡：图标、标题、一两句说明、按钮，排查信息收进详情。 */
export function EntryCard({
  actions,
  children,
  detail,
  details,
  icon,
  title,
  tone = 'neutral',
}: EntryCardProps) {
  const { t } = useTranslation();
  return (
    <main className={`entry-card entry-card--${tone}`} id="main-content">
      <section aria-live="polite" className="entry-card__body">
        {icon === undefined ? null : (
          <div aria-hidden="true" className="entry-card__icon">
            {icon}
          </div>
        )}
        <h1>{title}</h1>
        {detail === undefined ? null : <p className="entry-card__lede">{detail}</p>}
        {children}
        {actions === undefined ? null : <div className="entry-card__actions">{actions}</div>}
      </section>
      {details === undefined ? null : (
        <Details className="entry-card__details" summary={t('entry.details')}>
          {details}
        </Details>
      )}
    </main>
  );
}
