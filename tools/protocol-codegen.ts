import { mkdir, readFile, writeFile } from 'node:fs/promises';
import { dirname, resolve } from 'node:path';
import { fileURLToPath } from 'node:url';

type JsonObject = Readonly<Record<string, unknown>>;

const root = resolve(dirname(fileURLToPath(import.meta.url)), '..');
const schemaPath = resolve(root, 'packages/protocol/schema/v1/agent-room.schema.json');
const typescriptPath = resolve(root, 'packages/protocol-types/src/generated.ts');
const rustPath = resolve(root, 'crates/protocol-conformance/src/generated.rs');

function asObject(value: unknown, context: string): JsonObject {
  if (typeof value !== 'object' || value === null || Array.isArray(value)) {
    throw new TypeError(`${context} 必须是对象`);
  }

  return value as JsonObject;
}

function asString(value: unknown, context: string): string {
  if (typeof value !== 'string') {
    throw new TypeError(`${context} 必须是字符串`);
  }

  return value;
}

function stringArray(value: unknown, context: string): string[] {
  if (!Array.isArray(value)) {
    throw new TypeError(`${context} 必须是字符串数组`);
  }

  return value.map((item: unknown, index: number) =>
    asString(item, `${context}[${String(index)}]`),
  );
}

function refName(reference: string): string {
  const prefix = '#/$defs/';
  if (!reference.startsWith(prefix)) {
    throw new Error(`只允许当前协议文件内的引用：${reference}`);
  }

  return reference.slice(prefix.length);
}

function toSnakeCase(value: string): string {
  return value.replaceAll(/([a-z0-9])([A-Z])/g, '$1_$2').toLowerCase();
}

function toPascalCase(value: string): string {
  return value
    .split(/[^A-Za-z0-9]+/u)
    .filter(Boolean)
    .map((part) => `${part[0]?.toUpperCase() ?? ''}${part.slice(1)}`)
    .join('');
}

function unionReferences(value: unknown, context: string): string[] {
  if (!Array.isArray(value) || value.length < 2) {
    throw new TypeError(`${context} 必须至少包含两个联合成员`);
  }

  return value.map((member: unknown, index: number) => {
    const node = asObject(member, `${context}[${String(index)}]`);
    return refName(asString(node.$ref, `${context}[${String(index)}].$ref`));
  });
}

function unionVariantName(unionName: string, memberName: string): string {
  const withoutUnionSuffix = memberName.endsWith(unionName)
    ? memberName.slice(0, -unionName.length)
    : memberName;
  return withoutUnionSuffix.length > 0 ? withoutUnionSuffix : memberName;
}

function typescriptType(nodeValue: unknown): string {
  const node = asObject(nodeValue, '类型节点');
  if ('$ref' in node) {
    return refName(asString(node.$ref, '$ref'));
  }

  if ('oneOf' in node) {
    return unionReferences(node.oneOf, 'oneOf').join(' | ');
  }

  if ('const' in node) {
    return JSON.stringify(asString(node.const, 'const'));
  }

  const type = asString(node.type, 'type');
  if (type === 'string') {
    if ('enum' in node) {
      return stringArray(node.enum, 'enum')
        .map((item) => JSON.stringify(item))
        .join(' | ');
    }
    return 'string';
  }
  if (type === 'integer' || type === 'number') {
    return 'number';
  }
  if (type === 'boolean') {
    return 'boolean';
  }
  if (type === 'array') {
    return `ReadonlyArray<${typescriptType(node.items)}>`;
  }
  if (type === 'object') {
    return 'Readonly<Record<string, unknown>>';
  }

  throw new Error(`不支持的 TypeScript Schema 类型：${type}`);
}

function rustType(nodeValue: unknown): string {
  const node = asObject(nodeValue, '类型节点');
  if ('$ref' in node) {
    return refName(asString(node.$ref, '$ref'));
  }

  if ('const' in node) {
    return 'String';
  }

  const type = asString(node.type, 'type');
  if (type === 'string') {
    return 'String';
  }
  if (type === 'integer') {
    return typeof node.minimum === 'number' && node.minimum >= 0 ? 'u64' : 'i64';
  }
  if (type === 'number') {
    return 'f64';
  }
  if (type === 'boolean') {
    return 'bool';
  }
  if (type === 'array') {
    return `Vec<${rustType(node.items)}>`;
  }
  if (type === 'object') {
    return 'BTreeMap<String, serde_json::Value>';
  }

  throw new Error(`不支持的 Rust Schema 类型：${type}`);
}

function renderTypeScript(definitions: JsonObject): string {
  const blocks = Object.entries(definitions)
    .sort(([left], [right]) => left.localeCompare(right))
    .map(([name, definitionValue]) => {
      const definition = asObject(definitionValue, `定义 ${name}`);
      if (definition.type === 'string' && 'enum' in definition) {
        return `export type ${name} = ${typescriptType(definition)};`;
      }

      if ('oneOf' in definition) {
        return `export type ${name} = ${typescriptType(definition)};`;
      }

      if (definition.type !== 'object') {
        throw new Error(`顶层定义 ${name} 必须是对象或枚举`);
      }

      const properties = asObject(definition.properties, `${name}.properties`);
      const required = new Set(stringArray(definition.required ?? [], `${name}.required`));
      const fields = Object.entries(properties)
        .sort(([left], [right]) => left.localeCompare(right))
        .map(([field, node]) => {
          const optional = required.has(field) ? '' : '?';
          return `  readonly ${field}${optional}: ${typescriptType(node)};`;
        })
        .join('\n');
      const extension =
        definition.additionalProperties === true ? ' & Readonly<Record<string, unknown>>' : '';
      return `export type ${name} = {\n${fields}\n}${extension};`;
    });

  return `// 本文件由 tools/protocol-codegen.ts 生成，禁止手工修改。\n\n${blocks.join('\n\n')}\n`;
}

function renderRust(definitions: JsonObject): string {
  const blocks = Object.entries(definitions)
    .sort(([left], [right]) => left.localeCompare(right))
    .map(([name, definitionValue]) => {
      const definition = asObject(definitionValue, `定义 ${name}`);
      if (definition.type === 'string' && 'enum' in definition) {
        const variants = stringArray(definition.enum, `${name}.enum`)
          .map(
            (value) =>
              `    #[serde(rename = ${JSON.stringify(value)})]\n    ${toPascalCase(value)},`,
          )
          .join('\n');
        return `#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]\npub enum ${name} {\n${variants}\n}`;
      }

      if ('oneOf' in definition) {
        const variants = unionReferences(definition.oneOf, `${name}.oneOf`)
          .map((memberName) => `    ${unionVariantName(name, memberName)}(${memberName}),`)
          .join('\n');
        return `#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]\n#[serde(untagged)]\npub enum ${name} {\n${variants}\n}`;
      }

      if (definition.type !== 'object') {
        throw new Error(`顶层定义 ${name} 必须是对象或枚举`);
      }

      const properties = asObject(definition.properties, `${name}.properties`);
      const required = new Set(stringArray(definition.required ?? [], `${name}.required`));
      const fields = Object.entries(properties)
        .sort(([left], [right]) => left.localeCompare(right))
        .map(([field, node]) => {
          const rustField = toSnakeCase(field);
          const type = rustType(node);
          if (required.has(field)) {
            return `    pub ${rustField}: ${type},`;
          }
          return `    #[serde(default, skip_serializing_if = "Option::is_none")]\n    pub ${rustField}: Option<${type}>,`;
        });

      if (definition.additionalProperties === true) {
        fields.push(
          '    #[serde(default, flatten, skip_serializing_if = "BTreeMap::is_empty")]\n    pub extensions: BTreeMap<String, serde_json::Value>,',
        );
      }

      return `#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]\n#[serde(rename_all = "camelCase")]\npub struct ${name} {\n${fields.join('\n')}\n}`;
    });

  return `// 本文件由 tools/protocol-codegen.ts 生成，禁止手工修改。\n\nuse std::collections::BTreeMap;\n\nuse serde::{Deserialize, Serialize};\n\n${blocks.join('\n\n')}\n`;
}

async function writeOrCheck(path: string, content: string, check: boolean): Promise<void> {
  if (check) {
    let current: string;
    try {
      current = await readFile(path, 'utf8');
    } catch {
      throw new Error(`缺少生成文件：${path}`);
    }
    if (current !== content) {
      throw new Error(`生成文件已漂移：${path}`);
    }
    return;
  }

  await mkdir(dirname(path), { recursive: true });
  await writeFile(path, content, 'utf8');
}

const parsed: unknown = JSON.parse(await readFile(schemaPath, 'utf8'));
const schema = asObject(parsed, '协议 Schema');
const definitions = asObject(schema.$defs, '$defs');
const check = process.argv.includes('--check');

await Promise.all([
  writeOrCheck(typescriptPath, renderTypeScript(definitions), check),
  writeOrCheck(rustPath, renderRust(definitions), check),
]);

process.stdout.write(check ? '协议生成物一致。\n' : '协议类型已重新生成。\n');
