import { execFileSync } from 'node:child_process';

const corepackRequirement =
  process.platform === 'win32'
    ? {
        command: process.env.ComSpec ?? 'cmd.exe',
        args: ['/d', '/s', '/c', 'corepack pnpm@10.28.0 --version'],
      }
    : { command: 'corepack', args: ['pnpm@10.28.0', '--version'] };

const requirements = [
  { command: 'git', args: ['--version'], label: 'Git' },
  { command: 'cargo', args: ['--version'], label: 'Cargo' },
  { command: 'node', args: ['--version'], label: 'Node.js 24' },
  {
    ...corepackRequirement,
    label: 'pnpm 10（由 Corepack 固定）',
  },
  { command: 'docker', args: ['compose', 'version'], label: 'Docker Compose 2' },
];

let failed = false;

for (const requirement of requirements) {
  try {
    const output = execFileSync(requirement.command, requirement.args, {
      encoding: 'utf8',
      stdio: ['ignore', 'pipe', 'pipe'],
    }).trim();
    process.stdout.write(`✓ ${requirement.label}: ${output}\n`);
  } catch (error) {
    failed = true;
    const reason = error instanceof Error ? error.message : String(error);
    process.stderr.write(`✗ ${requirement.label}: ${reason}\n`);
  }
}

if (failed) {
  process.exitCode = 1;
} else {
  process.stdout.write('开发工具链检查通过。\n');
}
