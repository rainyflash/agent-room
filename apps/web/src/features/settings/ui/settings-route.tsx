import { SecuritySettings } from '@/features/security/ui/security-page';
import { SettingsLayout, SettingsSectionContent } from './settings-page';
import type { SettingsSection } from './settings-sections';

/** 路由上的设置页：安全一节装上真实的安全服务，其余分节自己取数据。 */
export function SettingsRoute({ section }: { readonly section: SettingsSection }) {
  return (
    <SettingsLayout section={section}>
      {section === 'security' ? <SecuritySettings /> : <SettingsSectionContent section={section} />}
    </SettingsLayout>
  );
}
