import { spawnSync } from 'node:child_process';
import { resolve } from 'node:path';

const [script, ...arguments_] = process.argv.slice(2);
if (script === undefined) {
  throw new Error('必须提供 PowerShell 脚本路径。');
}

const candidates = process.platform === 'win32' ? ['pwsh', 'powershell.exe'] : ['pwsh'];
const executable = candidates.find((candidate) => {
  const probe = spawnSync(
    candidate,
    ['-NoLogo', '-NoProfile', '-Command', '$PSVersionTable.PSVersion'],
    {
      stdio: 'ignore',
    },
  );
  return probe.error === undefined && probe.status === 0;
});

if (executable === undefined) {
  throw new Error('找不到可用的 PowerShell 运行时。');
}
const result = spawnSync(
  executable,
  ['-NoLogo', '-NoProfile', '-ExecutionPolicy', 'Bypass', '-File', resolve(script), ...arguments_],
  { stdio: 'inherit' },
);

if (result.error !== undefined) {
  throw result.error;
}

process.exitCode = result.status ?? 1;
