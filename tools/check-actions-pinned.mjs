import { readFileSync, readdirSync } from 'node:fs';
import { URL } from 'node:url';

const workflowsDirectory = new URL('../.github/workflows/', import.meta.url);
const workflowNames = readdirSync(workflowsDirectory)
  .filter((name) => name.endsWith('.yml') || name.endsWith('.yaml'))
  .sort();
const remoteActionPattern = /^[^/\s]+\/[^@\s]+@[a-f0-9]{40}$/u;
const findings = [];
let remoteActionCount = 0;

for (const workflowName of workflowNames) {
  const lines = readFileSync(new URL(workflowName, workflowsDirectory), 'utf8').split(/\r?\n/u);

  lines.forEach((line, index) => {
    const match = /^\s*uses:\s*([^#\s]+)(?:\s*#.*)?$/u.exec(line);
    if (!match) {
      return;
    }

    const action = match[1];
    if (!action || action.startsWith('./') || action.startsWith('docker://')) {
      return;
    }

    remoteActionCount += 1;
    if (!remoteActionPattern.test(action)) {
      findings.push(`${workflowName}:${index + 1}: ${action}`);
    }
  });
}

if (remoteActionCount === 0) {
  throw new Error('未找到任何远程 GitHub Action，无法验证固定策略。');
}

if (findings.length > 0) {
  process.stderr.write(`发现未固定到完整提交 SHA 的 Action：\n${findings.join('\n')}\n`);
  process.exitCode = 1;
} else {
  process.stdout.write(`GitHub Actions 固定检查通过，共验证 ${remoteActionCount} 个引用。\n`);
}
