import type { BrowserGateway } from '@/features/session/domain/session';
import { authenticationCallbackFailure } from '@/features/session/domain/authentication-callback';

export function safeInternalPath(value: string | null): string | null {
  if (value === null || !value.startsWith('/') || value.startsWith('//') || value.includes('\\')) {
    return null;
  }
  return value;
}

export class WindowBrowserGateway implements BrowserGateway {
  currentPath(): string {
    const current = new URL(window.location.href);
    if (
      current.pathname === '/connect' &&
      authenticationCallbackFailure(`${current.pathname}${current.search}`) === null
    ) {
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
    const destination = safeInternalPath(path) ?? '/connect';
    const current = `${window.location.pathname}${window.location.search}${window.location.hash}`;
    if (destination !== current) window.location.replace(destination);
  }
}
