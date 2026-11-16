import { createContext, useContext, useEffect, useSyncExternalStore, type ReactNode } from 'react';

import { onWindowForegroundChange } from '@/shared/browser/window-foreground';

import {
  browserWebNotifications,
  type WebNotifications,
  type WebNotificationState,
} from '../adapters/web-notifications';

const WebNotificationsContext = createContext<WebNotifications | null>(null);
let shared: WebNotifications | null = null;

/** 测试和夹具换成假的；应用里不用包，直接用这个浏览器自己的。 */
export function WebNotificationsProvider({
  children,
  value,
}: {
  readonly children: ReactNode;
  readonly value: WebNotifications;
}) {
  return (
    <WebNotificationsContext.Provider value={value}>{children}</WebNotificationsContext.Provider>
  );
}

export function useWebNotifications(): WebNotifications {
  const provided = useContext(WebNotificationsContext);
  if (provided !== null) return provided;
  shared ??= browserWebNotifications();
  return shared;
}

/** 开关和权限的当前状态；回到页面时重读权限：人可能刚在浏览器的网站设置里改过。 */
export function useWebNotificationState(): {
  readonly notifications: WebNotifications;
  readonly state: WebNotificationState;
} {
  const notifications = useWebNotifications();
  const state = useSyncExternalStore(
    notifications.subscribe,
    notifications.state,
    notifications.state,
  );
  useEffect(() => {
    notifications.refresh();
    return onWindowForegroundChange(() => {
      notifications.refresh();
    });
  }, [notifications]);
  return { notifications, state };
}
