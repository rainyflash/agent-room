import type { BrowserGateway } from '@/features/session/domain/session';

export function safeInternalPath(value: string | null): string | null {
  if (value === null || !value.startsWith('/') || value.startsWith('//') || value.includes('\\')) {
    return null;
  }
  return value;
}

export class WindowBrowserGateway implements BrowserGateway {
  currentPath(): string {
    const current = new URL(window.location.href);
    if (current.pathname === '/connect') {
      const requested = safeInternalPath(current.searchParams.get('returnTo'));
      if (requested !== null) {
        return requested;
      }
    }
    current.searchParams.delete('loginToken');
    return `${current.pathname}${current.search}${current.hash}`;
  }

  isOnline(): boolean {
    return window.navigator.onLine;
  }

  replacePath(path: string): void {
    window.location.replace(safeInternalPath(path) ?? '/connect');
  }
}
