/**
 * 网页通知（specs/redesign/reminders.md 4R-2）：网页开着、标签页不在前面时，和桌面端一样弹系统通知。
 * 开关和浏览器的权限都只在这个浏览器里，不同步、不经过服务器；通知的字在这台设备上解开、在这台设备上弹。
 */
export type WebNotificationPermission = 'unsupported' | 'default' | 'granted' | 'denied';

export type WebNotificationState = {
  readonly permission: WebNotificationPermission;
  /** 人在设置里或提示上打开过。浏览器没给权限时照样记着，权限给了就生效。 */
  readonly wanted: boolean;
  /** 问过一次“要不要在后台也提醒你”，答了（打开、不用了、关掉）就不再问。 */
  readonly offered: boolean;
};

export type WebNotificationNote = {
  readonly title: string;
  readonly body: string;
  /** 同一个标记的新通知顶掉旧的，不一条条堆起来。 */
  readonly tag: string;
  readonly onClick: () => void;
};

export type WebNotifications = {
  readonly state: () => WebNotificationState;
  readonly subscribe: (listener: () => void) => () => void;
  /** 打开时浏览器还没问过就先问；回答新的状态。 */
  readonly setWanted: (wanted: boolean) => Promise<WebNotificationState>;
  readonly markOffered: () => void;
  /** 打开了、浏览器也给了权限才弹；弹了回答 true。 */
  readonly show: (note: WebNotificationNote) => boolean;
  /** 人可能在浏览器的网站设置里改了权限：回到页面时重读一次。 */
  readonly refresh: () => void;
};

export function webNotificationsActive(state: WebNotificationState): boolean {
  return state.wanted && state.permission === 'granted';
}

type NotificationInstance = { onclick: ((event: Event) => void) | null; close(): void };
export type NotificationApi = {
  readonly permission: string;
  requestPermission(): Promise<string>;
  new (title: string, options: { body: string; tag: string; icon: string }): NotificationInstance;
};

type KeyValueStorage = Pick<Storage, 'getItem' | 'setItem'>;

const wantedKey = 'agent-room.web-notifications.v1';
const offeredKey = 'agent-room.web-notifications.offered.v1';
const icon = '/icons/192x192.png';

export class BrowserWebNotifications implements WebNotifications {
  readonly #api: NotificationApi | null;
  readonly #storage: KeyValueStorage | null;
  readonly #listeners = new Set<() => void>();
  #snapshot: WebNotificationState;

  constructor(api: NotificationApi | null, storage: KeyValueStorage | null) {
    this.#api = api;
    this.#storage = storage;
    this.#snapshot = this.#read();
  }

  state = (): WebNotificationState => this.#snapshot;

  subscribe = (listener: () => void): (() => void) => {
    this.#listeners.add(listener);
    return () => {
      this.#listeners.delete(listener);
    };
  };

  setWanted = async (wanted: boolean): Promise<WebNotificationState> => {
    this.#write(wantedKey, wanted ? 'on' : 'off');
    if (wanted && this.#api !== null && this.#api.permission === 'default') {
      try {
        await this.#api.requestPermission();
      } catch {
        // 老浏览器只认回调式的写法，或者被浏览器拦了：照现在的权限算。
      }
    }
    this.refresh();
    return this.#snapshot;
  };

  markOffered = (): void => {
    this.#write(offeredKey, 'yes');
    this.refresh();
  };

  show = (note: WebNotificationNote): boolean => {
    const api = this.#api;
    if (api === null || !webNotificationsActive(this.#read())) return false;
    try {
      const notification = new api(note.title, { body: note.body, icon, tag: note.tag });
      notification.onclick = () => {
        notification.close();
        note.onClick();
      };
      return true;
    } catch {
      // 安卓上的 Chrome 只许经 Service Worker 弹，直接 new 会抛错：那里只有页面里的提示。
      return false;
    }
  };

  refresh = (): void => {
    const next = this.#read();
    const current = this.#snapshot;
    if (
      next.permission === current.permission &&
      next.wanted === current.wanted &&
      next.offered === current.offered
    )
      return;
    this.#snapshot = next;
    for (const listener of this.#listeners) listener();
  };

  #read(): WebNotificationState {
    return {
      offered: this.#get(offeredKey) === 'yes',
      permission: permissionOf(this.#api),
      wanted: this.#get(wantedKey) === 'on',
    };
  }

  #get(key: string): string | null {
    try {
      return this.#storage?.getItem(key) ?? null;
    } catch {
      return null;
    }
  }

  #write(key: string, value: string): void {
    try {
      this.#storage?.setItem(key, value);
    } catch {
      // 不让存（隐私模式、关了网站数据）：这一次照样生效，下次打开再问。
    }
  }
}

function permissionOf(api: NotificationApi | null): WebNotificationPermission {
  if (api === null) return 'unsupported';
  const permission = api.permission;
  return permission === 'granted' || permission === 'denied' ? permission : 'default';
}

/** 这个浏览器自己的：没有 Notification（iPhone 上没加到主屏幕的网页）时算不支持。 */
export function browserWebNotifications(): BrowserWebNotifications {
  const api =
    typeof globalThis.Notification === 'function'
      ? (globalThis.Notification as unknown as NotificationApi)
      : null;
  return new BrowserWebNotifications(api, localStorageOrNull());
}

/** 关了网站数据的浏览器里，读 localStorage 本身就会抛错。 */
function localStorageOrNull(): KeyValueStorage | null {
  try {
    return globalThis.localStorage;
  } catch {
    return null;
  }
}
