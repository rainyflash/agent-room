import { execFileSync } from 'node:child_process';
import { readFileSync } from 'node:fs';
import { extname } from 'node:path';

const patterns = [
  { name: '私钥', expression: /-----BEGIN (?:EC |RSA |OPENSSH )?PRIVATE KEY-----/u },
  { name: 'OpenAI 密钥', expression: /\bsk-(?:proj-)?[A-Za-z0-9_-]{32,}\b/u },
  { name: 'AWS 访问密钥', expression: /\b(?:AKIA|ASIA)[A-Z0-9]{16}\b/u },
  {
    name: '长 JWT',
    expression: /\beyJ[A-Za-z0-9_-]{16,}\.[A-Za-z0-9_-]{16,}\.[A-Za-z0-9_-]{16,}\b/u,
  },
  {
    name: '硬编码敏感值',
    expression:
      /\b(?:client_secret|access_token|refresh_token|private_key|api_key)\b\s*[:=]\s*["'][A-Za-z0-9_+/=-]{24,}["']/iu,
  },
];

const textExtensions = new Set([
  '',
  '.json',
  '.md',
  '.mjs',
  '.ps1',
  '.rs',
  '.sh',
  '.toml',
  '.ts',
  '.txt',
  '.yaml',
  '.yml',
]);
const ignoredFiles = new Set(['tools/check-secrets.mjs']);

function findingsFor(text) {
  return patterns.filter(({ expression }) => expression.test(text)).map(({ name }) => name);
}

if (process.argv.includes('--self-test')) {
  const syntheticSecret = `sk-${'x'.repeat(48)}`;
  if (!findingsFor(syntheticSecret).includes('OpenAI 密钥')) {
    throw new Error('Secret 扫描器自检失败：未识别合成密钥。');
  }
  if (findingsFor('KEYCLOAK_CLIENT_SECRET=generated-locally').length !== 0) {
    throw new Error('Secret 扫描器自检失败：误报环境模板。');
  }
  process.stdout.write('Secret 扫描器自检通过。\n');
  process.exit(0);
}

const files = execFileSync('git', ['ls-files', '--cached', '--others', '--exclude-standard'], {
  encoding: 'utf8',
})
  .split(/\r?\n/u)
  .filter(Boolean)
  .filter((path) => !ignoredFiles.has(path))
  .filter((path) => textExtensions.has(extname(path).toLowerCase()));

const findings = [];
for (const path of files) {
  const text = readFileSync(path, 'utf8');
  for (const name of findingsFor(text)) {
    findings.push(`${path}: ${name}`);
  }
}

if (findings.length > 0) {
  process.stderr.write(`发现疑似凭据：\n${findings.map((item) => `- ${item}`).join('\n')}\n`);
  process.exitCode = 1;
} else {
  process.stdout.write(`Secret 扫描通过，共检查 ${files.length} 个文本文件。\n`);
}
