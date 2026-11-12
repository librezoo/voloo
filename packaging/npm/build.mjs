// Builds the npm packages for this machine's platform into dist/npm and packs them.
// Usage: node packaging/npm/build.mjs [--skip-build]
// Other platforms are built the same way on their own machines (or CI) and published together.
import { execFileSync } from 'node:child_process';
import { copyFileSync, chmodSync, mkdirSync, readFileSync, rmSync, writeFileSync } from 'node:fs';
import { dirname, join, resolve } from 'node:path';
import { fileURLToPath } from 'node:url';

const root = resolve(dirname(fileURLToPath(import.meta.url)), '../..');
const cargo = readFileSync(join(root, 'Cargo.toml'), 'utf8');
const version = cargo.match(/^version\s*=\s*"([^"]+)"/m)[1];
const target = `${process.platform}-${process.arch}`;
const exeName = process.platform === 'win32' ? 'voloo.exe' : 'voloo';
const out = join(root, 'dist', 'npm');

if (!process.argv.includes('--skip-build')) execFileSync('cargo', ['build', '--release', '--locked'], { cwd: root, stdio: 'inherit' });
rmSync(out, { recursive: true, force: true });

const common = {
  version,
  description: 'Control USB-C Voloo Switch devices: CLI, MCP server for AI agents, local HTTP API and web dashboard.',
  homepage: 'https://github.com/librezoo/voloo',
  repository: { type: 'git', url: 'git+https://github.com/librezoo/voloo.git' },
  license: 'Apache-2.0',
};

// Platform package: just the executable.
const platDir = join(out, `cli-${target}`);
mkdirSync(join(platDir, 'bin'), { recursive: true });
copyFileSync(join(root, 'target', 'release', exeName), join(platDir, 'bin', exeName));
chmodSync(join(platDir, 'bin', exeName), 0o755);
copyFileSync(join(root, 'LICENSE'), join(platDir, 'LICENSE'));
writeFileSync(join(platDir, 'package.json'), JSON.stringify({
  name: `@librezoo/voloo-${target}`, ...common,
  os: [process.platform], cpu: [process.arch], files: ['bin'], preferUnplugged: true,
}, null, 2) + '\n');

// Main package: the launcher, depending on every platform package optionally.
const targets = ['darwin-arm64', 'darwin-x64', 'linux-x64', 'linux-arm64', 'win32-x64'];
const mainDir = join(out, 'voloo');
mkdirSync(join(mainDir, 'bin'), { recursive: true });
copyFileSync(join(root, 'packaging', 'npm', 'voloo', 'bin', 'voloo.js'), join(mainDir, 'bin', 'voloo.js'));
chmodSync(join(mainDir, 'bin', 'voloo.js'), 0o755);
copyFileSync(join(root, 'README.md'), join(mainDir, 'README.md'));
copyFileSync(join(root, 'LICENSE'), join(mainDir, 'LICENSE'));
writeFileSync(join(mainDir, 'package.json'), JSON.stringify({
  name: '@librezoo/voloo', ...common,
  bin: { voloo: 'bin/voloo.js' },
  files: ['bin', 'README.md'],
  engines: { node: '>=18' },
  keywords: ['voloo', 'librezoo', 'power', 'usb', 'mcp', 'ai-agent'],
  optionalDependencies: Object.fromEntries(targets.map((t) => [`@librezoo/voloo-${t}`, version])),
}, null, 2) + '\n');

for (const dir of [platDir, mainDir]) execFileSync('npm', ['pack', '--pack-destination', out], { cwd: dir, stdio: 'inherit', shell: process.platform === 'win32' }); // npm is npm.cmd on Windows
console.log(`npm packages for ${target} in ${out}`);
