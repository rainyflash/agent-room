import { execFileSync } from 'node:child_process';
import { copyFile, readFile } from 'node:fs/promises';
import { createRequire } from 'node:module';
import path from 'node:path';
import { fileURLToPath, URL } from 'node:url';

const root = fileURLToPath(new URL('../', import.meta.url));
const source = path.join(root, 'apps/web/public/agent-room-mark.svg');
const svg = await readFile(source, 'utf8');
const color = svg.match(/<svg\b[^>]*\bcolor="(#[\da-f]{6})"/iu)?.[1];
if (color === undefined) throw new Error('The source SVG must declare its primary brand color.');

const desktopRequire = createRequire(new URL('../apps/desktop/package.json', import.meta.url));
const cli = path.join(
  path.dirname(desktopRequire.resolve('@tauri-apps/cli/package.json')),
  'tauri.js',
);
const desktopIcons = path.join(root, 'apps/desktop/src-tauri/icons');

function generateIcons(output, extraArgs = []) {
  execFileSync(process.execPath, [cli, 'icon', source, '--output', output, ...extraArgs], {
    cwd: root,
    stdio: 'inherit',
  });
}

generateIcons(desktopIcons, ['--ios-color', color]);
generateIcons(path.join(root, 'apps/web/public/icons'), [
  '--png',
  '180',
  '--png',
  '192',
  '--png',
  '512',
]);
await copyFile(source, path.join(desktopIcons, 'agent-room-mark.svg'));
// 登录页（Keycloak 主题）的标志和标签页图标也用同一个文件。
await copyFile(
  source,
  path.join(root, 'infra/identity/themes/agent-room/login/resources/img/agent-room-mark.svg'),
);
process.stdout.write('Brand assets synchronized from apps/web/public/agent-room-mark.svg.\n');
