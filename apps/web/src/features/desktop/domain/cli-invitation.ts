import { z } from 'zod';
import type { AgentInviteIdentity } from './agent-invite';
import { agentInviteHosts, inviteIdentityStorageKey, readInviteIdentity } from './agent-invite';
import type { DesktopRuntimeSnapshot } from './desktop-runtime';

const identitySchema = z
  .object({
    sessionKey: z.uuidv7(),
    displayName: z
      .string()
      .trim()
      .min(1)
      .refine((value) => Array.from(value).length <= 128)
      .refine((value) => !/[\p{Cc}]/u.test(value)),
    ownerId: z.string().nullable(),
    room: z
      .object({
        roomId: z.string().min(1).max(512),
        roomName: z.string().min(1).max(512),
        catalogId: z.uuidv7().optional(),
      })
      .strict()
      .transform(({ catalogId, ...value }) => ({
        ...value,
        ...(catalogId === undefined ? {} : { catalogId }),
      }))
      .nullable()
      .optional(),
  })
  .strict()
  .transform(({ room, ...value }) => ({ ...value, ...(room === undefined ? {} : { room }) }));
const historySchema = z.array(identitySchema).max(64);
const historyKey = (ownerId: string) => `agent-room.invitations.v2.${ownerId}`;

export type InviteHistory = {
  readonly identities: readonly AgentInviteIdentity[];
  readonly unavailable: boolean;
};

export function readInviteHistory(
  storage: Pick<Storage, 'getItem'>,
  ownerId: string | null,
): InviteHistory {
  if (ownerId === null) return { identities: [], unavailable: false };
  try {
    const raw = storage.getItem(historyKey(ownerId));
    const parsed = raw === null ? [] : historySchema.parse(JSON.parse(raw));
    const identities = parsed.filter((entry) => entry.ownerId === ownerId);
    // Previously one identity was saved per tool. Offer those explicitly for restoration;
    // never pick them automatically for a new task or claim an unowned legacy identity.
    for (const host of agentInviteHosts) {
      const legacy = readInviteIdentity(storage, inviteIdentityStorageKey(host), null);
      if (
        legacy?.ownerId === ownerId &&
        !identities.some((entry) => entry.sessionKey === legacy.sessionKey)
      )
        identities.push(legacy);
    }
    return { identities, unavailable: false };
  } catch {
    return { identities: [], unavailable: true };
  }
}

export function saveInviteHistory(
  storage: Pick<Storage, 'getItem' | 'setItem'>,
  identity: AgentInviteIdentity,
): boolean {
  if (identity.ownerId === null) return false;
  const history = readInviteHistory(storage, identity.ownerId);
  if (history.unavailable) return false;
  const entries = [
    identity,
    ...history.identities.filter((entry) => entry.sessionKey !== identity.sessionKey),
  ].slice(0, 64);
  try {
    storage.setItem(historyKey(identity.ownerId), JSON.stringify(entries));
    return true;
  } catch {
    return false;
  }
}

export function encodeCliInvitation(
  identity: AgentInviteIdentity,
  roomId: string | null,
  catalogId?: string,
): string {
  const bytes = new TextEncoder().encode(
    JSON.stringify({
      version: 1,
      sessionKey: identity.sessionKey,
      displayName: identity.displayName,
      roomId,
      ...(catalogId === undefined ? {} : { catalogId }),
    }),
  );
  return btoa(String.fromCharCode(...bytes))
    .replaceAll('+', '-')
    .replaceAll('/', '_')
    .replace(/=+$/u, '');
}

export function cliInvocation(
  configuration: DesktopRuntimeSnapshot['cliConfiguration'],
  platform: DesktopRuntimeSnapshot['platform'],
): string {
  if (configuration === null || configuration === undefined) return 'agent-room';
  const quote = (value: string) =>
    platform === 'windows'
      ? `'${value.replaceAll("'", "''")}'`
      : `'${value.replaceAll("'", "'\"'\"'")}'`;
  return `${platform === 'windows' ? '& ' : ''}${[configuration.command, ...configuration.args].map(quote).join(' ')}`;
}
