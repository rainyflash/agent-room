import { z } from 'zod';
import { err, ok, type Result } from '@/shared/result';

const timestamp = z.number().int().nonnegative().max(Number.MAX_SAFE_INTEGER);
const agentId = z.uuid();
const roomId = z
  .string()
  .min(4)
  .max(512)
  .regex(/^![^\s:]+:[^\s]+$/u);
const cursor = z.object({ eventId: z.string().min(1).max(512), timestamp }).strict();
const revision = { clock: timestamp, writerId: z.string().min(1).max(255) };
const changes = [
  z.object({ kind: z.literal('favorite'), id: agentId, value: z.boolean() }).strict(),
  z
    .object({
      kind: z.literal('tags'),
      id: agentId,
      value: z
        .array(z.string().trim().min(1).max(32))
        .max(12)
        .refine((tags) => new Set(tags).size === tags.length),
    })
    .strict(),
  z.object({ kind: z.literal('recent'), id: agentId, value: timestamp }).strict(),
  z.object({ kind: z.literal('read'), id: roomId, value: cursor }).strict(),
  z
    .object({ kind: z.literal('acknowledged'), id: z.string().min(1).max(512), value: timestamp })
    .strict(),
  z.object({ kind: z.literal('mute'), id: roomId, value: z.boolean() }).strict(),
  z
    .object({
      kind: z.literal('position'),
      id: roomId,
      value: z
        .object({
          messageId: z.uuid(),
          offset: z.number().min(-10000).max(10000),
          following: z.boolean(),
        })
        .strict(),
    })
    .strict(),
  z
    .object({ kind: z.literal('dnd'), id: z.literal('global'), value: timestamp.nullable() })
    .strict(),
] as const;

const changeSchema = z.discriminatedUnion('kind', changes);
const [firstChange, ...remainingChanges] = changes;
const entrySchema = z.discriminatedUnion('kind', [
  firstChange.extend(revision),
  ...remainingChanges.map((change) => change.extend(revision)),
]);
const maximumEntries = 10000;
const documentSchema = z
  .object({ schema: z.literal(1), entries: z.array(entrySchema).max(maximumEntries) })
  .strict()
  .refine((document) => new Set(document.entries.map(entryKey)).size === document.entries.length);

export type WorkspaceChange = z.infer<typeof changeSchema>;
export type WorkspaceEntry = z.infer<typeof entrySchema>;
export type WorkspaceDocument = { readonly schema: 1; readonly entries: readonly WorkspaceEntry[] };
export type ReadingCursor = z.infer<typeof cursor>;
export type ReadingPosition = Extract<WorkspaceChange, { kind: 'position' }>['value'];
export type WorkspaceFailure = { readonly code: string; readonly retryable: boolean };
export const emptyWorkspace: WorkspaceDocument = Object.freeze({
  schema: 1,
  entries: Object.freeze([]),
});

export function parseWorkspace(input: unknown): Result<WorkspaceDocument, WorkspaceFailure> {
  const parsed = documentSchema.safeParse(input);
  return parsed.success
    ? ok(freezeDocument(parsed.data.entries))
    : err({ code: 'workspace.invalid_document', retryable: false });
}

export function changeWorkspace(
  document: WorkspaceDocument,
  change: WorkspaceChange,
  writerId: string,
): Result<WorkspaceDocument, WorkspaceFailure> {
  const validated = changeSchema.safeParse(change);
  const clock = document.entries.reduce((maximum, entry) => Math.max(maximum, entry.clock), 0) + 1;
  const candidate = entrySchema.safeParse({ ...validated.data, clock, writerId });
  if (!validated.success || !candidate.success)
    return err({ code: 'workspace.invalid_change', retryable: false });
  const previous = document.entries.find((entry) => entryKey(entry) === entryKey(candidate.data));
  if (
    previous !== undefined &&
    JSON.stringify(previous.value) === JSON.stringify(candidate.data.value)
  )
    return ok(document);
  return mergeWorkspaces(document, freezeDocument([candidate.data]));
}

export function mergeWorkspaces(
  left: WorkspaceDocument,
  right: WorkspaceDocument,
): Result<WorkspaceDocument, WorkspaceFailure> {
  const entries = new Map(left.entries.map((entry) => [entryKey(entry), entry]));
  for (const incoming of right.entries) {
    const key = entryKey(incoming);
    const current = entries.get(key);
    if (current === undefined || compareEntries(incoming, current) > 0) entries.set(key, incoming);
  }
  if (entries.size > maximumEntries)
    return err({ code: 'workspace.storage_limit', retryable: false });
  return ok(freezeDocument([...entries.values()]));
}

export function workspaceDocumentsEqual(
  left: WorkspaceDocument,
  right: WorkspaceDocument,
): boolean {
  return left === right || JSON.stringify(left) === JSON.stringify(right);
}

export function compareCursors(left: ReadingCursor, right: ReadingCursor): number {
  return left.timestamp - right.timestamp || compareText(left.eventId, right.eventId);
}

export function workspaceIndex(document: WorkspaceDocument) {
  const favorites = new Set<string>();
  const tags = new Map<string, readonly string[]>();
  const recent = new Map<string, number>();
  const read = new Map<string, ReadingCursor>();
  const acknowledged = new Set<string>();
  const muted = new Set<string>();
  const positions = new Map<string, ReadingPosition>();
  let doNotDisturbUntil: number | null = 0;
  for (const entry of document.entries) {
    switch (entry.kind) {
      case 'favorite':
        if (entry.value) favorites.add(entry.id);
        break;
      case 'tags':
        tags.set(entry.id, entry.value);
        break;
      case 'recent':
        recent.set(entry.id, entry.value);
        break;
      case 'read':
        read.set(entry.id, entry.value);
        break;
      case 'acknowledged':
        acknowledged.add(entry.id);
        break;
      case 'mute':
        if (entry.value) muted.add(entry.id);
        break;
      case 'position':
        positions.set(entry.id, entry.value);
        break;
      case 'dnd':
        doNotDisturbUntil = entry.value;
        break;
    }
  }
  return { favorites, tags, recent, read, acknowledged, muted, positions, doNotDisturbUntil };
}

export type WorkspaceIndex = ReturnType<typeof workspaceIndex>;

function entryKey(entry: Pick<WorkspaceEntry, 'kind' | 'id'>): string {
  return JSON.stringify([entry.kind, entry.id]);
}

function compareEntries(left: WorkspaceEntry, right: WorkspaceEntry): number {
  // Read progress and last interaction never move backwards when an old device reconnects.
  if (left.kind === 'read' && right.kind === 'read') {
    const order = compareCursors(left.value, right.value);
    if (order !== 0) return order;
  }
  if (
    (left.kind === 'recent' && right.kind === 'recent') ||
    (left.kind === 'acknowledged' && right.kind === 'acknowledged')
  ) {
    const order = left.value - right.value;
    if (order !== 0) return order;
  }
  return (
    left.clock - right.clock ||
    compareText(left.writerId, right.writerId) ||
    compareText(JSON.stringify(left.value), JSON.stringify(right.value))
  );
}

function compareText(left: string, right: string): number {
  return left === right ? 0 : left < right ? -1 : 1;
}

function freezeDocument(entries: readonly WorkspaceEntry[]): WorkspaceDocument {
  for (const entry of entries)
    if (typeof entry.value === 'object' && entry.value !== null) Object.freeze(entry.value);
  return Object.freeze({
    schema: 1,
    entries: Object.freeze(
      entries
        .toSorted((left, right) => compareText(entryKey(left), entryKey(right)))
        .map((entry) => Object.freeze(entry)),
    ),
  });
}
