/** 设置的分节。单独放一个文件，路由判断地址时不用把整个设置页打进入口包。 */
export const settingsSections = [
  'general',
  'account',
  'security',
  'this-computer',
  'about',
] as const;
export type SettingsSection = (typeof settingsSections)[number];

export function isSettingsSection(value: string): value is SettingsSection {
  return (settingsSections as readonly string[]).includes(value);
}
