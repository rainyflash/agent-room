import 'i18next';

import type { resources } from '@/shared/i18n/resources';

declare module 'i18next' {
  // eslint-disable-next-line @typescript-eslint/consistent-type-definitions -- i18next 依赖接口声明合并扩展类型。
  interface CustomTypeOptions {
    defaultNS: 'translation';
    enableSelector: false;
    resources: {
      translation: (typeof resources)['en']['translation'];
    };
    returnNull: false;
  }
}
