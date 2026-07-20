import { readFile } from 'node:fs/promises';
import { resolve } from 'node:path';
import { fileURLToPath } from 'node:url';

export function validateReleaseTag({ refType, refName, version }) {
  if (refType !== 'tag') {
    return { tagged: false, expected: `v${version}` };
  }
  const expected = `v${version}`;
  if (refName !== expected) {
    throw new Error(`Release tag ${refName} does not match package version ${expected}`);
  }
  return { tagged: true, expected };
}

const isMain = process.argv[1] && resolve(process.argv[1]) === fileURLToPath(import.meta.url);
if (isMain) {
  const root = resolve(import.meta.dirname, '..');
  const manifest = JSON.parse(await readFile(resolve(root, 'package.json'), 'utf8'));
  const result = validateReleaseTag({
    refType: process.env.GITHUB_REF_TYPE,
    refName: process.env.GITHUB_REF_NAME,
    version: manifest.version,
  });
  console.log(
    result.tagged
      ? `Verified release tag ${result.expected}.`
      : `Manual release build for ${result.expected}; no tag will be published.`,
  );
}
