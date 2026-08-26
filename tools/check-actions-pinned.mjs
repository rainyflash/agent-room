import { readFileSync, readdirSync } from 'node:fs';
import { URL } from 'node:url';

const workflowsDirectory = new URL('../.github/workflows/', import.meta.url);
const workflowNames = readdirSync(workflowsDirectory)
  .filter((name) => name.endsWith('.yml') || name.endsWith('.yaml'))
  .sort();
const remoteActionPattern = /^[^/\s]+\/[^@\s]+@[a-f0-9]{40}$/u;
const hostedMacosPattern = /\bmacos-(?:latest|\d+)(?:-(?:large|xlarge|intel))?\b/giu;
const unpinnedActions = [];
const hostedMacosRunners = [];
let remoteActionCount = 0;

for (const workflowName of workflowNames) {
  const lines = readFileSync(new URL(workflowName, workflowsDirectory), 'utf8').split(/\r?\n/u);

  lines.forEach((line, index) => {
    const executableLine = line.split('#', 1)[0] ?? '';
    const hostedMacosMatches = [...executableLine.matchAll(hostedMacosPattern)];
    hostedMacosMatches.forEach((match) => {
      hostedMacosRunners.push(`${workflowName}:${index + 1}: ${match[0]}`);
    });

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
      unpinnedActions.push(`${workflowName}:${index + 1}: ${action}`);
    }
  });
}

if (remoteActionCount === 0) {
  throw new Error('未找到任何远程 GitHub Action，无法验证固定策略。');
}

if (unpinnedActions.length > 0) {
  process.stderr.write(`发现未固定到完整提交 SHA 的 Action：\n${unpinnedActions.join('\n')}\n`);
  process.exitCode = 1;
}

if (hostedMacosRunners.length > 0) {
  process.stderr.write(
    `禁止使用昂贵的 GitHub 托管 macOS Runner；请改用手动自托管 macOS：\n${hostedMacosRunners.join('\n')}\n`,
  );
  process.exitCode = 1;
}

if (process.exitCode === undefined) {
  process.stdout.write(
    `GitHub Actions 策略检查通过：${remoteActionCount} 个远程 Action 均已固定，且未配置托管 macOS。\n`,
  );
}
