import { TriangleAlert } from 'lucide-react';
import { useTranslation } from 'react-i18next';

import { EntryCard, EntryShell } from '@/shared/ui/entry-shell';

export type ConfigurationFailureProps = {
  readonly issues: readonly string[];
};

/** 构建时写进去的连接配置有误：应用起不来，只能请用户更新。具体哪一项不对收进详情。 */
export function ConfigurationFailure({ issues }: ConfigurationFailureProps) {
  const { t } = useTranslation();
  return (
    <EntryShell home={false}>
      <EntryCard
        detail={t('config.description')}
        details={
          <ul>
            {issues.map((issue) => (
              <li key={issue}>{issue}</li>
            ))}
          </ul>
        }
        icon={<TriangleAlert />}
        title={t('config.title')}
        tone="alert"
      />
    </EntryShell>
  );
}
