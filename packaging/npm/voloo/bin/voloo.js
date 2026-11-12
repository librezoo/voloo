#!/usr/bin/env node
// Launcher for the native `voloo` executable shipped in @librezoo/voloo-<platform>-<arch>.
// It only forwards arguments, stdio (MCP speaks over stdio) and the exit code; all logic
// lives in the native program, which also starts the background process itself.
'use strict';
const { spawnSync } = require('node:child_process');
const path = require('node:path');

const target = `${process.platform}-${process.arch}`;
const pkg = `@librezoo/voloo-${target}`;
let exe;
try {
  const dir = path.dirname(require.resolve(`${pkg}/package.json`));
  exe = path.join(dir, 'bin', process.platform === 'win32' ? 'voloo.exe' : 'voloo');
} catch {
  console.error(`voloo: no native build is installed for ${target}.\n` +
    `Reinstall with optional dependencies enabled (npm install @librezoo/voloo), or download a binary for your platform.`);
  process.exit(4);
}
const result = spawnSync(exe, process.argv.slice(2), { stdio: 'inherit', windowsHide: true });
if (result.error) {
  console.error(`voloo: cannot start ${exe}: ${result.error.message}`);
  process.exit(3);
}
if (result.signal) process.kill(process.pid, result.signal);
process.exit(result.status ?? 1);
