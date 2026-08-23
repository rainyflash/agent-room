import Ajv2020 from 'ajv/dist/2020.js';
import addFormats from 'ajv-formats';
import type { AnySchema } from 'ajv';

export type ProtocolValidationResult =
  { readonly ok: true } | { readonly ok: false; readonly details: string };

export type ProtocolValidator = (candidate: unknown) => ProtocolValidationResult;

function isSchema(value: unknown): value is AnySchema {
  return typeof value === 'boolean' || (typeof value === 'object' && value !== null);
}

export function createProtocolValidator(schema: unknown): ProtocolValidator {
  if (!isSchema(schema)) {
    throw new TypeError('协议 Schema 必须是对象或布尔值。');
  }

  const ajv = new Ajv2020.default({ allErrors: true, strict: true });
  addFormats.default(ajv);

  if (!ajv.validateSchema(schema)) {
    throw new Error(`协议 Schema 无效：${ajv.errorsText(ajv.errors)}`);
  }

  const validate = ajv.compile(schema);
  return (candidate: unknown): ProtocolValidationResult => {
    if (validate(candidate)) {
      return { ok: true };
    }

    return { ok: false, details: ajv.errorsText(validate.errors) };
  };
}
