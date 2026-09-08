import { execFileSync } from 'node:child_process';
import { copyFile, readFile, writeFile } from 'node:fs/promises';
import { createRequire } from 'node:module';
import path from 'node:path';
import { fileURLToPath, URL } from 'node:url';

import { format, resolveConfig } from 'prettier';

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
await copyFile(source, path.join(root, 'plugins/agent-room/assets/agent-room-mark.svg'));

const pluginPath = path.join(root, 'plugins/agent-room/.codex-plugin/plugin.json');
const plugin = JSON.parse(await readFile(pluginPath, 'utf8'));
plugin.interface.brandColor = color;
await writeFile(
  pluginPath,
  await format(JSON.stringify(plugin), {
    ...(await resolveConfig(pluginPath)),
    filepath: pluginPath,
  }),
);
process.stdout.write('Brand assets synchronized from apps/web/public/agent-room-mark.svg.\n');
