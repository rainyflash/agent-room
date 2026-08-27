import { copyFile, mkdir } from 'node:fs/promises';
import { basename, dirname, join, resolve } from 'node:path';
import { fileURLToPath } from 'node:url';
import { spawnSync } from 'node:child_process';

const toolDirectory = dirname(fileURLToPath(import.meta.url));
const repositoryRoot = resolve(toolDirectory, '..', '..', '..');
const release = process.argv.includes('--release');
const profile = release ? 'release' : 'debug';

const rustc = run('rustc', ['-vV']);
const hostLine = rustc.stdout.split(/\r?\n/u).find((line) => line.startsWith('host: '));
if (hostLine === undefined) {
  throw new Error('无法从 rustc -vV 解析目标三元组。');
}
const targetTriple = hostLine.slice('host: '.length).trim();
if (!/^[a-z0-9_.-]+$/u.test(targetTriple)) {
  throw new Error(`rustc 返回了不安全的目标三元组：${targetTriple}`);
}

const sidecars = [
  { packageName: 'agent-room-bridge', executableName: 'agent-room-bridge' },
  { packageName: 'agent-room-mcp', executableName: 'agent-room-mcp' },
];
const cargoArguments = ['build'];
for (const sidecar of sidecars) {
  cargoArguments.push('-p', sidecar.packageName);
}
if (release) {
  cargoArguments.push('--release');
}
run('cargo', cargoArguments, { inherit: true });

const destinationDirectory = resolve(toolDirectory, '..', 'src-tauri', 'binaries');
await mkdir(destinationDirectory, { recursive: true });
for (const sidecar of sidecars) {
  const executableName =
    process.platform === 'win32' ? `${sidecar.executableName}.exe` : sidecar.executableName;
  const source = join(repositoryRoot, 'target', profile, executableName);
  const destination = join(
    destinationDirectory,
    process.platform === 'win32'
      ? `${sidecar.executableName}-${targetTriple}.exe`
      : `${sidecar.executableName}-${targetTriple}`,
  );
  await copyFile(source, destination);
  process.stdout.write(`已准备桌面 sidecar：${basename(destination)}\n`);
}

function run(command, args, options = {}) {
  const result = spawnSync(command, args, {
    cwd: repositoryRoot,
    encoding: 'utf8',
    stdio: options.inherit === true ? 'inherit' : ['ignore', 'pipe', 'pipe'],
  });
  if (result.status !== 0) {
    const diagnostic = options.inherit === true ? '' : `\n${result.stderr.trim()}`;
    throw new Error(`${command} ${args.join(' ')} 执行失败。${diagnostic}`);
  }
  return {
    stdout: typeof result.stdout === 'string' ? result.stdout : '',
  };
}
