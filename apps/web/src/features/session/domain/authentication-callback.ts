export type AuthenticationCallbackFailure = 'expired' | 'failed';

export function authenticationCallbackFailure(path: string): AuthenticationCallbackFailure | null {
  if (!/^\/connect(?:[?#]|$)/u.test(path)) return null;
  const query = new URLSearchParams(path.split('?')[1]?.split('#')[0]);
  const reason = query.get('authentication');
  return reason === 'expired' || reason === 'failed' ? reason : null;
}
