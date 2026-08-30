export function formatWorkspaceTime(timestamp: number, language: string | undefined): string {
  return new Intl.DateTimeFormat(language ?? 'en', {
    dateStyle: 'medium',
    timeStyle: 'short',
  }).format(timestamp);
}
