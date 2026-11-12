import assert from 'node:assert/strict';
import test from 'node:test';
import { publishPackage } from './publish.mjs';

const version = '0.7.0-rc.1';
const registryError = (code) => Object.assign(new Error(code), { stdout: JSON.stringify({ error: { code } }) });

test('a partial release skips an existing version', () => {
  const calls = [];
  publishPackage('@librezoo/voloo', version, 'main.tgz', (command, args) => {
    calls.push([command, ...args]);
    return JSON.stringify(version);
  });
  assert.deepEqual(calls, [['npm', 'view', `@librezoo/voloo@${version}`, 'version', '--json']]);
});

test('only a missing version is published, with the correct release tag', () => {
  for (const [release, tag] of [[version, 'next'], ['0.7.0', 'latest']]) {
    const calls = [];
    publishPackage('@librezoo/voloo', release, 'main.tgz', (command, args) => {
      calls.push([command, ...args]);
      if (args[0] === 'view') throw registryError('E404');
    });
    assert.deepEqual(calls[1], ['npm', 'publish', 'main.tgz', '--access', 'public', '--tag', tag]);
  }
});

test('authentication and network errors stop the release without publishing', () => {
  for (const error of [registryError('E401'), registryError('E403'), registryError('E503'), new Error('ETIMEDOUT')]) {
    let calls = 0;
    assert.throws(() => publishPackage('@librezoo/voloo', version, 'main.tgz', () => {
      calls++;
      throw error;
    }), (actual) => actual === error);
    assert.equal(calls, 1);
  }
});

test('a rejected publish stops the release', () => {
  const error = registryError('E403');
  assert.throws(() => publishPackage('@librezoo/voloo', version, 'main.tgz', (command, args) => {
    throw args[0] === 'view' ? registryError('E404') : error;
  }), (actual) => actual === error);
});
