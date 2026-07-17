import { createHash } from 'node:crypto';
import { readFile, readdir, writeFile } from 'node:fs/promises';
import { basename, resolve } from 'node:path';
import { fileURLToPath } from 'node:url';

const root = resolve(import.meta.dirname, '..');

export function inspectPe(buffer, label = 'artifact') {
  if (buffer.length < 64 || buffer[0] !== 0x4d || buffer[1] !== 0x5a) {
    throw new Error(`${label} is not a Portable Executable (missing MZ header)`);
  }
  const peOffset = buffer.readUInt32LE(0x3c);
  if (
    peOffset + 6 > buffer.length ||
    buffer[peOffset] !== 0x50 ||
    buffer[peOffset + 1] !== 0x45 ||
    buffer[peOffset + 2] !== 0 ||
    buffer[peOffset + 3] !== 0
  ) {
    throw new Error(`${label} is not a Portable Executable (missing PE signature)`);
  }
  const machine = buffer.readUInt16LE(peOffset + 4);
  if (machine !== 0x8664) {
    throw new Error(`${label} targets machine 0x${machine.toString(16)}, expected x64`);
  }
  return { machine, size: buffer.length };
}

export function checksumLine(path, buffer) {
  const digest = createHash('sha256').update(buffer).digest('hex');
  return `${digest}  ${basename(path)}`;
}

export function inspectZip(buffer, label = 'archive') {
  if (
    buffer.length < 4 ||
    buffer[0] !== 0x50 ||
    buffer[1] !== 0x4b ||
    ![
      [0x03, 0x04],
      [0x05, 0x06],
      [0x07, 0x08],
    ].some(([third, fourth]) => buffer[2] === third && buffer[3] === fourth)
  ) {
    throw new Error(`${label} is not a ZIP archive`);
  }
  return { size: buffer.length };
}

export function listZipEntries(buffer, label = 'archive') {
  inspectZip(buffer, label);

  // ZIP comments are limited to 65,535 bytes, so the end record must be in
  // this bounded suffix. Reading the central directory avoids OS-specific
  // `tar`/`unzip` behavior and does not inflate untrusted archive contents.
  const minimumRecordSize = 22;
  const searchStart = Math.max(0, buffer.length - minimumRecordSize - 0xffff);
  let endOffset = -1;
  for (let offset = buffer.length - minimumRecordSize; offset >= searchStart; offset -= 1) {
    if (buffer.readUInt32LE(offset) === 0x06054b50) {
      endOffset = offset;
      break;
    }
  }
  if (endOffset < 0) throw new Error(`${label} is missing its ZIP central directory`);

  const commentLength = buffer.readUInt16LE(endOffset + 20);
  if (endOffset + minimumRecordSize + commentLength !== buffer.length) {
    throw new Error(`${label} has a malformed ZIP end record`);
  }
  const diskNumber = buffer.readUInt16LE(endOffset + 4);
  const centralDisk = buffer.readUInt16LE(endOffset + 6);
  const diskEntries = buffer.readUInt16LE(endOffset + 8);
  const totalEntries = buffer.readUInt16LE(endOffset + 10);
  if (diskNumber !== 0 || centralDisk !== 0 || diskEntries !== totalEntries) {
    throw new Error(`${label} uses an unsupported multi-disk ZIP layout`);
  }

  const centralSize = buffer.readUInt32LE(endOffset + 12);
  const centralOffset = buffer.readUInt32LE(endOffset + 16);
  if (centralOffset + centralSize !== endOffset) {
    throw new Error(`${label} has a malformed ZIP central directory`);
  }

  const entries = [];
  let offset = centralOffset;
  for (let index = 0; index < totalEntries; index += 1) {
    if (offset + 46 > endOffset || buffer.readUInt32LE(offset) !== 0x02014b50) {
      throw new Error(`${label} has a malformed ZIP directory entry`);
    }
    const nameLength = buffer.readUInt16LE(offset + 28);
    const extraLength = buffer.readUInt16LE(offset + 30);
    const entryCommentLength = buffer.readUInt16LE(offset + 32);
    const nextOffset = offset + 46 + nameLength + extraLength + entryCommentLength;
    if (nextOffset > endOffset) {
      throw new Error(`${label} has a truncated ZIP directory entry`);
    }
    entries.push(buffer.toString('utf8', offset + 46, offset + 46 + nameLength));
    offset = nextOffset;
  }
  if (offset !== endOffset) throw new Error(`${label} has unparsed ZIP directory data`);
  return entries;
}

export async function verifyRelease({ pePaths, zipPaths, checksumPaths, checksumPath }) {
  for (const path of pePaths) {
    inspectPe(await readFile(path), basename(path));
  }
  for (const path of zipPaths) {
    inspectZip(await readFile(path), basename(path));
  }

  const lines = [];
  for (const path of checksumPaths) {
    const buffer = await readFile(path);
    lines.push(checksumLine(path, buffer));
  }
  lines.sort();
  await writeFile(checksumPath, `${lines.join('\n')}\n`, 'utf8');
  return lines;
}

const isMain = process.argv[1] && resolve(process.argv[1]) === fileURLToPath(import.meta.url);
if (isMain) {
  const pePaths = [
    resolve(root, 'release/BeatblockTogetherInstaller.exe'),
    resolve(root, 'artifacts/obs/beatblock-together-obs.dll'),
    resolve(root, 'artifacts/lovely/version.dll'),
  ];
  const modDirectory = resolve(root, 'mod/releases');
  const zipPaths = (await readdir(modDirectory))
    .filter((name) => name.endsWith('.zip'))
    .map((name) => resolve(modDirectory, name));
  if (zipPaths.length !== 2) {
    throw new Error(`Expected two mod release ZIPs, found ${zipPaths.length}`);
  }
  const checksumPath = resolve(root, 'release/SHA256SUMS.txt');
  const lines = await verifyRelease({
    pePaths,
    zipPaths,
    checksumPaths: [pePaths[0], pePaths[1], ...zipPaths],
    checksumPath,
  });
  console.log(`Verified ${pePaths.length} x64 PE artifacts and ${zipPaths.length} ZIP archives.`);
  console.log(lines.join('\n'));
  console.log(`Wrote ${checksumPath}`);
}
