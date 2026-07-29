import assert from 'node:assert/strict';
import { mkdtemp, mkdir, rm, writeFile } from 'node:fs/promises';
import { tmpdir } from 'node:os';
import { join, resolve } from 'node:path';
import { test } from 'node:test';
import { validateReleaseDocumentation, validateReleaseNoteText } from './verify-release-docs.mjs';

const repositoryRoot = resolve(import.meta.dirname, '..');
const tag = 'v1.2.3-beta.4';
const fileName = `${tag}.md`;
const technicalUrl =
  'https://github.com/DupeisTaken/beatblock-online/blob/main/docs/changelogs/' + fileName;

function releaseNote(link = technicalUrl) {
  return `# Beatblock Online ${tag}

Summary.

## Download

Download the installer.

## Highlights

- Highlight.

## Upgrade

Install it.

## Compatibility and known limitations

This is a prerelease.

## Technical details

Read the [technical changelog](${link}).
`;
}

function changelog() {
  return `# ${tag} technical changelog

## Context and root causes

Context.

## Implementation details

Details.

## Migration and compatibility

Migration.

## Validation evidence

Evidence.

## Known limitations

Limit.
`;
}

async function documentationFixture({ withChangelog = true, withIndexEntry = true } = {}) {
  const root = await mkdtemp(join(tmpdir(), 'bbt-release-docs-'));
  const releaseDirectory = join(root, 'docs', 'releases');
  const changelogDirectory = join(root, 'docs', 'changelogs');
  await Promise.all([
    mkdir(releaseDirectory, { recursive: true }),
    mkdir(changelogDirectory, { recursive: true }),
  ]);
  await writeFile(join(releaseDirectory, fileName), releaseNote(), 'utf8');
  if (withChangelog) {
    await writeFile(join(changelogDirectory, fileName), changelog(), 'utf8');
  }
  const index = withIndexEntry
    ? `[Public](./${fileName}) [Technical](../changelogs/${fileName})`
    : '# Releases';
  await writeFile(join(releaseDirectory, 'index.md'), index, 'utf8');
  return root;
}

test('checked-in release documentation satisfies the publication contract', async () => {
  assert.deepEqual(await validateReleaseDocumentation(repositoryRoot), [
    'v0.3.0-alpha.3',
    'v0.3.0-beta.1',
    'v0.3.0-beta.2',
    'v0.3.0-beta.3',
    'v0.3.0-beta.4',
    'v0.3.0-beta.5',
  ]);
});

test('release notes reject repository-relative links that break in GitHub Releases', () => {
  assert.throws(
    () =>
      validateReleaseNoteText(
        fileName,
        releaseNote().replace('Summary.', 'Read the [setup guide](../setup.md).'),
      ),
    /unsafe in a GitHub Release body: \.\.\/setup\.md/,
  );
  assert.throws(
    () =>
      validateReleaseNoteText(
        fileName,
        releaseNote().replace('Summary.', 'Read the [setup guide][setup].\n\n[setup]: ../setup.md'),
      ),
    /unsafe in a GitHub Release body: \.\.\/setup\.md/,
  );
});

test('release documentation requires one technical changelog per public note', async (context) => {
  const root = await documentationFixture({ withChangelog: false });
  context.after(() => rm(root, { recursive: true, force: true }));
  await assert.rejects(
    validateReleaseDocumentation(root),
    /missing technical changelogs: v1\.2\.3-beta\.4\.md/,
  );
});

test('release index must link the public and technical documents', async (context) => {
  const root = await documentationFixture({ withIndexEntry: false });
  context.after(() => rm(root, { recursive: true, force: true }));
  await assert.rejects(
    validateReleaseDocumentation(root),
    /does not link both documents for v1\.2\.3-beta\.4/,
  );
});
