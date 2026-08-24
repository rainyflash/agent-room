import { readFileSync, readdirSync } from 'node:fs';
import { fileURLToPath } from 'node:url';

import { describe, expect, test } from 'vitest';

import { createProtocolValidator } from './validator.js';

const schemaPath = fileURLToPath(new URL('../schema/v1/agent-room.schema.json', import.meta.url));
const fixturesRoot = fileURLToPath(new URL('../fixtures/', import.meta.url));
const schema: unknown = JSON.parse(readFileSync(schemaPath, 'utf8'));
const validate = createProtocolValidator(schema);

function fixtureNames(kind: 'valid' | 'invalid'): string[] {
  return readdirSync(`${fixturesRoot}/${kind}`)
    .filter((name) => name.endsWith('.json'))
    .sort();
}

function readFixture(kind: 'valid' | 'invalid', name: string): unknown {
  return JSON.parse(readFileSync(`${fixturesRoot}/${kind}/${name}`, 'utf8'));
}

describe('协议 Schema', () => {
  for (const name of fixtureNames('valid')) {
    test(`接受正例 ${name}`, () => {
      const fixture = readFixture('valid', name);
      const result = validate(fixture);
      expect(result.ok, result.ok ? undefined : result.details).toBe(true);
    });
  }

  for (const name of fixtureNames('invalid')) {
    test(`拒绝反例 ${name}`, () => {
      const fixture = readFixture('invalid', name);
      expect(validate(fixture).ok).toBe(false);
    });
  }

  test('预览标题和摘要执行独立字符边界', () => {
    const fixture = readFixture('valid', 'message-preview.json');
    if (typeof fixture !== 'object' || fixture === null) {
      throw new Error('消息预览夹具必须是对象');
    }
    const preview = Reflect.get(fixture, 'preview');
    if (typeof preview !== 'object' || preview === null) {
      throw new Error('消息预览元数据必须是对象');
    }

    expect(validate({ ...fixture, preview: { ...preview, title: '界'.repeat(120) } }).ok).toBe(
      true,
    );
    expect(validate({ ...fixture, preview: { ...preview, title: '界'.repeat(121) } }).ok).toBe(
      false,
    );
    expect(validate({ ...fixture, preview: { ...preview, summary: '界'.repeat(500) } }).ok).toBe(
      true,
    );
    expect(validate({ ...fixture, preview: { ...preview, summary: '界'.repeat(501) } }).ok).toBe(
      false,
    );
  });

  test('状态协议接受完整枚举并拒绝粗粒度详情', () => {
    const fixture = readFixture('valid', 'agent-status.json');
    if (typeof fixture !== 'object' || fixture === null) {
      throw new Error('状态夹具必须是对象');
    }

    expect(validate({ ...fixture, status: 'waiting_input' }).ok).toBe(true);
    expect(validate({ ...fixture, status: 'completed' }).ok).toBe(true);
    expect(
      validate({
        ...fixture,
        visibility: 'coarse',
        taskSummary: '不得随粗粒度状态发布',
      }).ok,
    ).toBe(false);
  });

  test('拒绝非 Schema 输入', () => {
    expect(() => createProtocolValidator(null)).toThrow(TypeError);
  });

  test('拒绝不受支持的 Schema 关键字值', () => {
    expect(() => createProtocolValidator({ type: 'not-a-json-schema-type' })).toThrow(Error);
  });
});
