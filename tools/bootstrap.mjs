import { execFileSync } from 'node:child_process';
import { readFileSync } from 'node:fs';
import process from 'node:process';
import { URL } from 'node:url';

const jsonOutput = process.argv.includes('--json');
const checkOnly = jsonOutput || process.argv.includes('--check');
const packageManifest = JSON.parse(
  readFileSync(new URL('../package.json', import.meta.url), 'utf8'),
);
const rustToolchain = readFileSync(new URL('../rust-toolchain.toml', import.meta.url), 'utf8');
const nodeRecommendation = readFileSync(
  new URL('../.node-version', import.meta.url),
  'utf8',
).trim();
const justVersion = readFileSync(new URL('../.just-version', import.meta.url), 'utf8').trim();
const pnpmVersion = packageManifest.packageManager.replace(/^pnpm@/, '');
const rustVersion = /^channel\s*=\s*"([^"]+)"/m.exec(rustToolchain)?.[1];

if (!rustVersion) {
  throw new Error('rust-toolchain.toml does not declare a channel.');
}

const corepackRequirement =
  process.platform === 'win32'
    ? {
        command: process.env.ComSpec ?? 'cmd.exe',
        args: ['/d', '/s', '/c', `corepack pnpm@${pnpmVersion} --version`],
      }
    : { command: 'corepack', args: [`pnpm@${pnpmVersion}`, '--version'] };

const requirements = [
  {
    command: 'git',
    args: ['--version'],
    label: 'Git >= 2.40',
    validate: (output) => atLeast(versionFrom(output), [2, 40, 0]),
  },
  {
    command: 'rustc',
    args: ['--version'],
    label: `Rust ${rustVersion}`,
    validate: (output) => versionFrom(output).join('.') === rustVersion,
  },
  {
    command: 'cargo',
    args: ['--version'],
    label: `Cargo ${rustVersion}`,
    validate: (output) => versionFrom(output).join('.') === rustVersion,
  },
  {
    command: 'node',
    args: ['--version'],
    label: `Node.js 24 (recommended ${nodeRecommendation})`,
    validate: (output) => versionFrom(output)[0] === 24,
  },
  {
    ...corepackRequirement,
    label: `pnpm ${pnpmVersion} via Corepack`,
    validate: (output) => versionFrom(output).join('.') === pnpmVersion,
  },
  {
    command: 'docker',
    args: ['compose', 'version'],
    label: 'Docker Compose >= 2.20',
    validate: (output) => atLeast(versionFrom(output), [2, 20, 0]),
  },
];

const results = requirements.map(inspectRequirement);
if (results.every((result) => result.ok) && !checkOnly) {
  ensureJust();
}
results.push(
  inspectRequirement({
    command: 'just',
    args: ['--version'],
    label: `just ${justVersion}`,
    validate: (output) => versionFrom(output).join('.') === justVersion,
  }),
);
const failed = results.filter((result) => !result.ok);

if (jsonOutput) {
  process.stdout.write(`${JSON.stringify({ ok: failed.length === 0, results }, null, 2)}\n`);
} else {
  for (const result of results) {
    const symbol = result.ok ? '✓' : '✗';
    const stream = result.ok ? process.stdout : process.stderr;
    stream.write(`${symbol} ${result.label}: ${result.detail}\n`);
  }
}

if (failed.length > 0) {
  process.exitCode = 1;
} else if (checkOnly) {
  if (!jsonOutput) {
    process.stdout.write('Contributor environment check passed.\n');
  }
} else {
  prepareWorkspace();
}

function inspectRequirement(requirement) {
  try {
    const output = execFileSync(requirement.command, requirement.args, {
      encoding: 'utf8',
      stdio: ['ignore', 'pipe', 'pipe'],
    }).trim();
    const valid = requirement.validate(output);
    return {
      label: requirement.label,
      ok: valid,
      detail: valid ? output : `unsupported version: ${output}`,
    };
  } catch (error) {
    return {
      label: requirement.label,
      ok: false,
      detail: error instanceof Error ? error.message : String(error),
    };
  }
}

function versionFrom(output) {
  const match = /v?(\d+)\.(\d+)\.(\d+)/.exec(output);
  if (!match) {
    return [];
  }
  return match.slice(1).map(Number);
}

function atLeast(actual, minimum) {
  for (let index = 0; index < minimum.length; index += 1) {
    const difference = (actual[index] ?? -1) - minimum[index];
    if (difference !== 0) {
      return difference > 0;
    }
  }
  return true;
}

function prepareWorkspace() {
  const commands = [
    {
      command: process.platform === 'win32' ? (process.env.ComSpec ?? 'cmd.exe') : 'corepack',
      args:
        process.platform === 'win32'
          ? ['/d', '/s', '/c', `corepack pnpm@${pnpmVersion} install --frozen-lockfile`]
          : [`pnpm@${pnpmVersion}`, 'install', '--frozen-lockfile'],
      label: 'Install JavaScript dependencies',
    },
    {
      command: process.platform === 'win32' ? (process.env.ComSpec ?? 'cmd.exe') : 'corepack',
      args:
        process.platform === 'win32'
          ? ['/d', '/s', '/c', `corepack pnpm@${pnpmVersion} protocol:generate`]
          : [`pnpm@${pnpmVersion}`, 'protocol:generate'],
      label: 'Generate protocol bindings',
    },
    { command: 'cargo', args: ['fetch', '--locked'], label: 'Fetch Rust dependencies' },
  ];

  for (const command of commands) {
    process.stdout.write(`→ ${command.label}\n`);
    execFileSync(command.command, command.args, { stdio: 'inherit' });
  }
  process.stdout.write('Agent Room contributor workspace is ready.\n');
}

function ensureJust() {
  const installed = inspectRequirement({
    command: 'just',
    args: ['--version'],
    label: `just ${justVersion}`,
    validate: (output) => versionFrom(output).join('.') === justVersion,
  });
  if (installed.ok) {
    return;
  }
  process.stdout.write(`→ Install just ${justVersion}\n`);
  execFileSync('cargo', ['install', 'just', '--locked', '--version', justVersion], {
    stdio: 'inherit',
  });
}
