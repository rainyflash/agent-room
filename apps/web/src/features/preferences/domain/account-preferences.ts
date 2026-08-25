import { z } from 'zod';

import { err, ok, type Result } from '@/shared/result';

const writerIdSchema = z.string().trim().min(1).max(255);
const logicalClockSchema = z.number().int().min(0).max(Number.MAX_SAFE_INTEGER);

const languageRegisterSchema = preferenceRegisterSchema(z.enum(['system', 'en', 'zh-CN']));
const lobbyViewRegisterSchema = preferenceRegisterSchema(z.enum(['scene', 'list']));

const accountPreferencesSchema = z
  .object({
    fields: z
      .object({
        language: languageRegisterSchema,
        lobbyView: lobbyViewRegisterSchema,
      })
      .strict(),
    schemaVersion: z.literal(1),
  })
  .strict();

export type LanguagePreference = 'system' | 'en' | 'zh-CN';
export type LobbyViewPreference = 'scene' | 'list';

export type AccountPreferenceValues = {
  readonly language: LanguagePreference;
  readonly lobbyView: LobbyViewPreference;
};

export type PreferenceRegister<TValue extends string> = {
  readonly logicalClock: number;
  readonly value: TValue;
  readonly writerId: string;
};

export type AccountPreferencesDocument = {
  readonly fields: {
    readonly language: PreferenceRegister<LanguagePreference>;
    readonly lobbyView: PreferenceRegister<LobbyViewPreference>;
  };
  readonly schemaVersion: 1;
};

export type AccountPreferencesFailure = {
  readonly code:
    'preferences.clock_exhausted' | 'preferences.invalid_document' | 'preferences.invalid_writer';
  readonly retryable: boolean;
};

const languagePreferences: ReadonlySet<string> = new Set(['system', 'en', 'zh-CN']);

export function isLanguagePreference(value: string): value is LanguagePreference {
  return languagePreferences.has(value);
}

export function parseAccountPreferencesDocument(
  input: unknown,
): Result<AccountPreferencesDocument, AccountPreferencesFailure> {
  const parsed = accountPreferencesSchema.safeParse(input);
  return parsed.success
    ? ok(freezeDocument(parsed.data))
    : err({ code: 'preferences.invalid_document', retryable: false });
}

export function createAccountPreferencesDocument(
  values: AccountPreferenceValues,
  writerId: string,
): Result<AccountPreferencesDocument, AccountPreferencesFailure> {
  const parsedWriterId = writerIdSchema.safeParse(writerId);
  if (!parsedWriterId.success) {
    return err({ code: 'preferences.invalid_writer', retryable: false });
  }
  return ok(
    freezeDocument({
      fields: {
        language: register(values.language, parsedWriterId.data, 0),
        lobbyView: register(values.lobbyView, parsedWriterId.data, 0),
      },
      schemaVersion: 1,
    }),
  );
}

export function updateAccountPreference<TKey extends keyof AccountPreferenceValues>(
  document: AccountPreferencesDocument,
  key: TKey,
  value: AccountPreferenceValues[TKey],
  writerId: string,
): Result<AccountPreferencesDocument, AccountPreferencesFailure> {
  const parsedWriterId = writerIdSchema.safeParse(writerId);
  if (!parsedWriterId.success) {
    return err({ code: 'preferences.invalid_writer', retryable: false });
  }
  const nextClock = Math.max(
    document.fields.language.logicalClock,
    document.fields.lobbyView.logicalClock,
  );
  if (nextClock === Number.MAX_SAFE_INTEGER) {
    return err({ code: 'preferences.clock_exhausted', retryable: false });
  }
  return ok(
    freezeDocument({
      ...document,
      fields: {
        ...document.fields,
        [key]: register(value, parsedWriterId.data, nextClock + 1),
      },
    }),
  );
}

export function mergeAccountPreferencesDocuments(
  left: AccountPreferencesDocument,
  right: AccountPreferencesDocument,
): AccountPreferencesDocument {
  return freezeDocument({
    fields: {
      language: maximumRegister(left.fields.language, right.fields.language),
      lobbyView: maximumRegister(left.fields.lobbyView, right.fields.lobbyView),
    },
    schemaVersion: 1,
  });
}

export function valuesFromAccountPreferences(
  document: AccountPreferencesDocument,
): AccountPreferenceValues {
  return Object.freeze({
    language: document.fields.language.value,
    lobbyView: document.fields.lobbyView.value,
  });
}

export function accountPreferencesDocumentsEqual(
  left: AccountPreferencesDocument,
  right: AccountPreferencesDocument,
): boolean {
  return (
    registersEqual(left.fields.language, right.fields.language) &&
    registersEqual(left.fields.lobbyView, right.fields.lobbyView)
  );
}

function preferenceRegisterSchema<TValue extends z.ZodType<string>>(valueSchema: TValue) {
  return z
    .object({
      logicalClock: logicalClockSchema,
      value: valueSchema,
      writerId: writerIdSchema,
    })
    .strict();
}

function register<TValue extends string>(
  value: TValue,
  writerId: string,
  logicalClock: number,
): PreferenceRegister<TValue> {
  return Object.freeze({ logicalClock, value, writerId });
}

function maximumRegister<TValue extends string>(
  left: PreferenceRegister<TValue>,
  right: PreferenceRegister<TValue>,
): PreferenceRegister<TValue> {
  return compareRegisters(left, right) >= 0 ? left : right;
}

function compareRegisters<TValue extends string>(
  left: PreferenceRegister<TValue>,
  right: PreferenceRegister<TValue>,
): number {
  const clockOrder = compareNumbers(left.logicalClock, right.logicalClock);
  if (clockOrder !== 0) {
    return clockOrder;
  }
  const writerOrder = compareStrings(left.writerId, right.writerId);
  return writerOrder === 0 ? compareStrings(left.value, right.value) : writerOrder;
}

function compareNumbers(left: number, right: number): number {
  return left === right ? 0 : left < right ? -1 : 1;
}

function compareStrings(left: string, right: string): number {
  return left === right ? 0 : left < right ? -1 : 1;
}

function registersEqual<TValue extends string>(
  left: PreferenceRegister<TValue>,
  right: PreferenceRegister<TValue>,
): boolean {
  return (
    left.logicalClock === right.logicalClock &&
    left.value === right.value &&
    left.writerId === right.writerId
  );
}

function freezeDocument(document: AccountPreferencesDocument): AccountPreferencesDocument {
  return Object.freeze({
    fields: Object.freeze({
      language: register(
        document.fields.language.value,
        document.fields.language.writerId,
        document.fields.language.logicalClock,
      ),
      lobbyView: register(
        document.fields.lobbyView.value,
        document.fields.lobbyView.writerId,
        document.fields.lobbyView.logicalClock,
      ),
    }),
    schemaVersion: 1,
  });
}
