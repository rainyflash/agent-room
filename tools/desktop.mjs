import { spawnSync } from 'node:child_process';
import { closeSync, existsSync, openSync } from 'node:fs';
import { availableParallelism } from 'node:os';
import { resolve } from 'node:path';
import { fileURLToPath, URL } from 'node:url';

const root = fileURLToPath(new URL('../', import.meta.url));
const packages = ['agent-room-host-adapters', 'agent-room-desktop', 'agent-room-bridge'];
const cargoPackages = packages.flatMap((name) => ['-p', name]);
const pnpm = (...args) => ['corepack', 'pnpm@10.28.0', ...args];
const nativeChecks = [
  ['cargo', 'fmt', '--all', '--check'],
  [
    'cargo',
    'clippy',
    '--locked',
    ...cargoPackages,
    '--all-targets',
    '--all-features',
    '--',
    '-D',
    'warnings',
  ],
  ['cargo', 'test', '--locked', ...cargoPackages, '--all-features'],
];
const webChecks = [
  pnpm('format:check'),
  ['node', '--test', 'tools/tests/desktop-workflow.test.mjs'],
  pnpm('--filter', '@agent-room/protocol', 'build'),
  pnpm('lint'),
  pnpm('--filter', '@agent-room/web', 'typecheck'),
  pnpm('i18n:check'),
  pnpm(
    'exec',
    'vitest',
    'run',
    'apps/web/src/features/desktop',
    'apps/web/src/features/session',
    'apps/web/src/features/workspace/domain/connection-health.spec.ts',
  ),
  pnpm(
    '--filter',
    '@agent-room/web',
    'exec',
    'playwright',
    'test',
    'agent-access.e2e.ts',
    'matrix-session-vault.e2e.ts',
  ),
];

export function desktopPlan(mode) {
  switch (mode) {
    case 'dev':
    case 'preview':
      return [pnpm('--filter', '@agent-room/desktop', 'preview')];
    case 'native-check':
      return nativeChecks;
    case 'check':
      return [...nativeChecks, ...webChecks];
    case 'package':
      return [...nativeChecks, ...webChecks, pnpm('build:desktop')];
    case 'hosts':
      return [
        pnpm('--filter', '@agent-room/desktop', 'prepare:sidecar:debug'),
        ['cargo', 'build', '--locked', '-p', 'agent-room-desktop'],
        [
          resolve(
            root,
            'target',
            'debug',
            `agent-room-desktop${process.platform === 'win32' ? '.exe' : ''}`,
          ),
          '--check-hosts',
        ],
      ];
    default:
      throw new Error(
        'Use: node tools/desktop.mjs dev|preview|check|package|hosts|native-check [--plan]',
      );
  }
}

export function executePlan(plan, run) {
  for (const command of plan) {
    const status = run(command);
    if (status !== 0)
      throw new Error(`Stopped before packaging: ${command.join(' ')} (exit ${String(status)})`);
  }
}

function checkBuildFiles() {
  if (process.platform !== 'win32') return;
  for (const name of ['agent-room-desktop', 'agent-room-bridge', 'agent-room-mcp', 'agent-room']) {
    const path = resolve(root, 'target', 'debug', `${name}.exe`);
    if (!existsSync(path)) continue;
    try {
      // Opening without writing detects Windows' executable lock before Cargo
      // spends minutes compiling. Never terminate another process implicitly.
      closeSync(openSync(path, 'r+'));
    } catch (error) {
      throw new Error(
        `Cannot rebuild ${name}. Close the running desktop preview from its tray before starting checks, a preview, or packaging.`,
        { cause: error },
      );
    }
  }
}

function run(command) {
  process.stdout.write(`\n> ${command.join(' ')}\n`);
  let [executable, ...args] = command;
  if (process.platform === 'win32' && executable === 'corepack') {
    // Only repository-owned, fixed command tokens reach cmd.exe. Paths supplied
    // by a user must always be executed directly, never interpolated here.
    if (args.some((arg) => !/^[a-zA-Z0-9_@/.=:-]+$/u.test(arg)))
      throw new Error('Unsafe Corepack argument');
    executable = process.env.ComSpec ?? 'cmd.exe';
    args = ['/d', '/s', '/c', 'corepack', ...args];
  }
  const result = spawnSync(executable, args, {
    cwd: root,
    stdio: 'inherit',
    windowsHide: true,
    env: {
      ...process.env,
      CARGO_BUILD_JOBS: process.env.CARGO_BUILD_JOBS ?? String(Math.min(4, availableParallelism())),
      ...(command.includes('playwright')
        ? {
            AGENT_ROOM_E2E_REUSE_SERVER: '0',
            AGENT_ROOM_E2E_PORT: process.env.AGENT_ROOM_E2E_PORT ?? '14174',
            VITE_AGENT_ROOM_CONTROL_PLANE_URL: 'https://api.agent-room.localhost:18443',
            VITE_AGENT_ROOM_MATRIX_HOMESERVER_URL: 'https://matrix.agent-room.localhost:18443',
          }
        : {}),
    },
  });
  if (result.error) throw result.error;
  return result.status;
}

if (process.argv[1] && resolve(process.argv[1]) === fileURLToPath(import.meta.url)) {
  try {
    const [, , mode, option] = process.argv;
    if (option !== undefined && option !== '--plan') throw new Error('Unknown option');
    const plan = desktopPlan(mode);
    if (option === '--plan') process.stdout.write(`${JSON.stringify(plan, null, 2)}\n`);
    else {
      checkBuildFiles();
      executePlan(plan, run);
    }
  } catch (error) {
    process.stderr.write(`${error instanceof Error ? error.message : String(error)}\n`);
    process.exitCode = 1;
  }
}
