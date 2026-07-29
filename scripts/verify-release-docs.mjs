import { readFile, readdir } from 'node:fs/promises';
import { resolve } from 'node:path';
import { fileURLToPath } from 'node:url';

const releaseFilePattern = /^v\d+\.\d+\.\d+(?:-[0-9A-Za-z.-]+)?\.md$/;
const repository = 'https://github.com/DupeisTaken/beatblock-online';
const requiredReleaseSections = [
  'Download',
  'Highlights',
  'Upgrade',
  'Compatibility and known limitations',
  'Technical details',
];
const requiredChangelogSections = [
  'Context and root causes',
  'Implementation details',
  'Migration and compatibility',
  'Validation evidence',
  'Known limitations',
];

function markdownLinks(markdown) {
  return [
    ...[...markdown.matchAll(/!?\[[^\]]*]\(([^)\s]+)(?:\s+"[^"]*")?\)/g)].map((match) => match[1]),
    ...[...markdown.matchAll(/^\s*\[[^\]]+]:\s*(\S+)/gm)].map((match) => match[1]),
    ...[...markdown.matchAll(/\bhref\s*=\s*["']([^"']+)["']/gi)].map((match) => match[1]),
    ...[...markdown.matchAll(/<((?:https?:\/\/|\.{0,2}\/|#)[^ >]+)>/g)].map((match) => match[1]),
  ];
}

function requireSections(markdown, sections, label) {
  for (const section of sections) {
    if (!markdown.includes(`## ${section}`)) {
      throw new Error(`${label} is missing the "${section}" section`);
    }
  }
}

export function validateReleaseNoteText(fileName, markdown) {
  if (!releaseFilePattern.test(fileName)) {
    throw new Error(`Invalid versioned release-note filename: ${fileName}`);
  }
  const tag = fileName.slice(0, -3);
  if (!markdown.startsWith(`# Beatblock Online ${tag}\n`)) {
    throw new Error(`${fileName} must start with "# Beatblock Online ${tag}"`);
  }
  requireSections(markdown, requiredReleaseSections, fileName);
  if (markdown.length > 5_500) {
    throw new Error(`${fileName} is too long for a public release note (${markdown.length} bytes)`);
  }

  const technicalUrl = `${repository}/blob/main/docs/changelogs/${fileName}`;
  if (!markdown.includes(`](${technicalUrl})`)) {
    throw new Error(`${fileName} must link its matching technical changelog`);
  }
  for (const link of markdownLinks(markdown)) {
    if (!link.startsWith('https://') && !link.startsWith('#')) {
      throw new Error(
        `${fileName} contains a link that is unsafe in a GitHub Release body: ${link}`,
      );
    }
  }
  return tag;
}

export function validateTechnicalChangelogText(fileName, markdown) {
  if (!releaseFilePattern.test(fileName)) {
    throw new Error(`Invalid versioned technical-changelog filename: ${fileName}`);
  }
  const tag = fileName.slice(0, -3);
  if (!markdown.startsWith(`# ${tag} technical changelog\n`)) {
    throw new Error(`${fileName} must start with "# ${tag} technical changelog"`);
  }
  requireSections(markdown, requiredChangelogSections, fileName);
  return tag;
}

export async function validateReleaseDocumentation(root) {
  const releaseDirectory = resolve(root, 'docs/releases');
  const changelogDirectory = resolve(root, 'docs/changelogs');
  const [releaseEntries, changelogEntries, index] = await Promise.all([
    readdir(releaseDirectory, { withFileTypes: true }),
    readdir(changelogDirectory, { withFileTypes: true }),
    readFile(resolve(releaseDirectory, 'index.md'), 'utf8'),
  ]);
  const releaseFiles = releaseEntries
    .filter((entry) => entry.isFile() && releaseFilePattern.test(entry.name))
    .map((entry) => entry.name)
    .sort();
  const changelogFiles = changelogEntries
    .filter((entry) => entry.isFile() && releaseFilePattern.test(entry.name))
    .map((entry) => entry.name)
    .sort();

  if (releaseFiles.length === 0) throw new Error('No versioned release notes were found');
  if (JSON.stringify(releaseFiles) !== JSON.stringify(changelogFiles)) {
    const missingChangelogs = releaseFiles.filter((name) => !changelogFiles.includes(name));
    const orphanChangelogs = changelogFiles.filter((name) => !releaseFiles.includes(name));
    throw new Error(
      `Release documentation pairs do not match` +
        `\nmissing technical changelogs: ${missingChangelogs.join(', ') || '(none)'}` +
        `\norphan technical changelogs: ${orphanChangelogs.join(', ') || '(none)'}`,
    );
  }

  const tags = [];
  for (const fileName of releaseFiles) {
    const [releaseNote, changelog] = await Promise.all([
      readFile(resolve(releaseDirectory, fileName), 'utf8'),
      readFile(resolve(changelogDirectory, fileName), 'utf8'),
    ]);
    const releaseTag = validateReleaseNoteText(fileName, releaseNote);
    const changelogTag = validateTechnicalChangelogText(fileName, changelog);
    if (releaseTag !== changelogTag) {
      throw new Error(`${fileName} does not describe the same release in both documents`);
    }
    if (!index.includes(`](./${fileName})`) || !index.includes(`](../changelogs/${fileName})`)) {
      throw new Error(`docs/releases/index.md does not link both documents for ${releaseTag}`);
    }
    tags.push(releaseTag);
  }
  return tags;
}

const isMain = process.argv[1] && resolve(process.argv[1]) === fileURLToPath(import.meta.url);
if (isMain) {
  const root = resolve(import.meta.dirname, '..');
  const tags = await validateReleaseDocumentation(root);
  console.log(`Verified ${tags.length} public release note / technical changelog pairs.`);
}
