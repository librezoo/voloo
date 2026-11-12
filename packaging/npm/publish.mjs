// Resume partial releases using npm's existing package name/version uniqueness.
import { execFileSync } from 'node:child_process';
import { resolve } from 'node:path';
import { pathToFileURL } from 'node:url';

export function publishPackage(name, version, file, run = execFileSync) {
  const spec = `${name}@${version}`;
  try {
    const existing = JSON.parse(run('npm', ['view', spec, 'version', '--json'], { encoding: 'utf8', stdio: 'pipe' }));
    if (existing !== version) throw new Error(`Unexpected registry version for ${spec}: ${existing}`);
    console.log(`Already published: ${spec}`);
    return;
  } catch (error) {
    let code;
    try { code = JSON.parse(error.stdout?.toString()).error?.code; } catch { /* Not a registry JSON error. */ }
    if (code !== 'E404') throw error;
  }
  const tag = version.includes('-') ? 'next' : 'latest';
  run('npm', ['publish', file, '--access', 'public', '--tag', tag], { stdio: 'inherit' });
}

if (process.argv[1] && import.meta.url === pathToFileURL(resolve(process.argv[1])).href) {
  const version = process.argv[2];
  if (!version) throw new Error('Usage: node packaging/npm/publish.mjs VERSION');
  for (const target of ['darwin-arm64', 'darwin-x64', 'linux-arm64', 'linux-x64', 'win32-x64']) {
    publishPackage(`@librezoo/voloo-${target}`, version, `dist/npm/librezoo-voloo-${target}-${version}.tgz`);
  }
  publishPackage('@librezoo/voloo', version, `dist/npm/librezoo-voloo-${version}.tgz`);
}
