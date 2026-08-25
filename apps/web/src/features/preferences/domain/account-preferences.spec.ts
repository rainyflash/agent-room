import { describe, expect, it } from 'vitest';

import {
  accountPreferencesDocumentsEqual,
  createAccountPreferencesDocument,
  mergeAccountPreferencesDocuments,
  parseAccountPreferencesDocument,
  updateAccountPreference,
  valuesFromAccountPreferences,
  type AccountPreferenceValues,
  type AccountPreferencesDocument,
} from './account-preferences';

describe('跨设备账户偏好', () => {
  it('拒绝未知字段、非法枚举和不安全逻辑时钟', () => {
    expect(
      parseAccountPreferencesDocument({
        fields: {
          language: { logicalClock: 0, value: 'fr', writerId: 'DEVICE_A' },
          lobbyView: { logicalClock: 0, value: 'scene', writerId: 'DEVICE_A' },
        },
        schemaVersion: 1,
      }),
    ).toEqual({
      error: { code: 'preferences.invalid_document', retryable: false },
      ok: false,
    });

    expect(
      parseAccountPreferencesDocument({
        fields: {
          language: { logicalClock: Number.MAX_SAFE_INTEGER + 1, value: 'en', writerId: 'A' },
          lobbyView: { logicalClock: 0, value: 'scene', writerId: 'A' },
        },
        schemaVersion: 1,
        unexpected: true,
      }),
    ).toEqual({
      error: { code: 'preferences.invalid_document', retryable: false },
      ok: false,
    });
  });

  it('本地修改使用大于全部已见时钟的 Lamport 版本', () => {
    const initial = document('DEVICE_A');
    const language = updateAccountPreference(initial, 'language', 'zh-CN', 'DEVICE_A');
    expect(language.ok).toBe(true);
    if (!language.ok) {
      throw new Error('测试偏好更新失败。');
    }
    const lobbyView = updateAccountPreference(language.value, 'lobbyView', 'list', 'DEVICE_A');

    expect(lobbyView).toMatchObject({
      ok: true,
      value: {
        fields: {
          language: { logicalClock: 1, value: 'zh-CN', writerId: 'DEVICE_A' },
          lobbyView: { logicalClock: 2, value: 'list', writerId: 'DEVICE_A' },
        },
      },
    });
  });

  it('并发设备修改按时钟、设备标识和值形成确定全序', () => {
    const initial = document('DEVICE_A');
    const fromA = updated(initial, 'language', 'zh-CN', 'DEVICE_A');
    const fromB = updated(initial, 'language', 'en', 'DEVICE_B');
    const winner = mergeAccountPreferencesDocuments(fromA, fromB);

    expect(valuesFromAccountPreferences(winner).language).toBe('en');
    expect(mergeAccountPreferencesDocuments(fromB, fromA)).toEqual(winner);

    const sameWriterFirst = updated(initial, 'language', 'en', 'DEVICE_A');
    const sameWriterSecond = updated(initial, 'language', 'zh-CN', 'DEVICE_A');
    expect(
      valuesFromAccountPreferences(
        mergeAccountPreferencesDocuments(sameWriterFirst, sameWriterSecond),
      ).language,
    ).toBe('zh-CN');
  });

  it('合并满足交换、结合和幂等，且不同字段不会互相覆盖', () => {
    const initial = document('DEVICE_A');
    const language = updated(initial, 'language', 'zh-CN', 'DEVICE_B');
    const lobbyView = updated(initial, 'lobbyView', 'list', 'DEVICE_C');
    const laterLanguage = updated(language, 'language', 'en', 'DEVICE_D');

    const leftAssociated = mergeAccountPreferencesDocuments(
      mergeAccountPreferencesDocuments(language, lobbyView),
      laterLanguage,
    );
    const rightAssociated = mergeAccountPreferencesDocuments(
      language,
      mergeAccountPreferencesDocuments(lobbyView, laterLanguage),
    );

    expect(leftAssociated).toEqual(rightAssociated);
    expect(mergeAccountPreferencesDocuments(language, lobbyView)).toEqual(
      mergeAccountPreferencesDocuments(lobbyView, language),
    );
    expect(mergeAccountPreferencesDocuments(leftAssociated, leftAssociated)).toEqual(
      leftAssociated,
    );
    expect(valuesFromAccountPreferences(leftAssociated)).toEqual({
      language: 'en',
      lobbyView: 'list',
    });
    expect(accountPreferencesDocumentsEqual(leftAssociated, rightAssociated)).toBe(true);
  });

  it('逻辑时钟耗尽时失败关闭，不产生不可比较版本', () => {
    const exhausted = parseAccountPreferencesDocument({
      fields: {
        language: {
          logicalClock: Number.MAX_SAFE_INTEGER,
          value: 'en',
          writerId: 'DEVICE_A',
        },
        lobbyView: { logicalClock: 0, value: 'scene', writerId: 'DEVICE_A' },
      },
      schemaVersion: 1,
    });
    if (!exhausted.ok) {
      throw new Error('测试文档解析失败。');
    }

    expect(updateAccountPreference(exhausted.value, 'lobbyView', 'list', 'DEVICE_A')).toEqual({
      error: { code: 'preferences.clock_exhausted', retryable: false },
      ok: false,
    });
  });
});

function document(writerId: string): AccountPreferencesDocument {
  const result = createAccountPreferencesDocument(
    { language: 'system', lobbyView: 'scene' },
    writerId,
  );
  if (!result.ok) {
    throw new Error('测试文档创建失败。');
  }
  return result.value;
}

function updated<TKey extends keyof AccountPreferenceValues>(
  source: AccountPreferencesDocument,
  key: TKey,
  value: AccountPreferenceValues[TKey],
  writerId: string,
): AccountPreferencesDocument {
  const result = updateAccountPreference(source, key, value, writerId);
  if (!result.ok) {
    throw new Error('测试偏好更新失败。');
  }
  return result.value;
}
