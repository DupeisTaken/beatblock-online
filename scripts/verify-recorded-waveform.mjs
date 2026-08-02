import { spawn } from 'node:child_process';
import { writeFile } from 'node:fs/promises';
import { resolve } from 'node:path';
import { fileURLToPath } from 'node:url';

const DEFAULT_SAMPLE_RATE = 48_000;
const DEFAULT_CHANNELS = 2;

function goertzelAmplitude(samples, sampleRate, targetHz, channel, channels) {
  const count = Math.floor(samples.length / channels);
  const radians = (2 * Math.PI * targetHz) / sampleRate;
  const coefficient = 2 * Math.cos(radians);
  let previous = 0;
  let previousPrevious = 0;
  for (let frame = 0; frame < count; frame += 1) {
    const current = samples[frame * channels + channel] + coefficient * previous - previousPrevious;
    previousPrevious = previous;
    previous = current;
  }
  const power = Math.max(
    0,
    previousPrevious ** 2 + previous ** 2 - coefficient * previous * previousPrevious,
  );
  return count > 0 ? (2 * Math.sqrt(power)) / count : 0;
}

/**
 * Distinguish a merely present audio stream from delivered, nonzero PCM. The
 * optional tone check makes the physical OBS probe deterministic instead of
 * accepting encoder noise or an unrelated system sound.
 */
export function analyzePcmFloat32(
  samples,
  { sampleRate = DEFAULT_SAMPLE_RATE, channels = DEFAULT_CHANNELS, targetHz } = {},
) {
  if (!(samples instanceof Float32Array) || samples.length === 0 || samples.length % channels !== 0)
    throw new Error('PCM must be a non-empty interleaved Float32Array');
  let squareSum = 0;
  let peak = 0;
  let nonzeroSamples = 0;
  for (const sample of samples) {
    const magnitude = Math.abs(sample);
    squareSum += sample * sample;
    if (magnitude > peak) peak = magnitude;
    if (magnitude > 1e-6) nonzeroSamples += 1;
  }
  const rms = Math.sqrt(squareSum / samples.length);
  let targetAmplitude = null;
  if (targetHz != null) {
    targetAmplitude = 0;
    for (let channel = 0; channel < channels; channel += 1)
      targetAmplitude = Math.max(
        targetAmplitude,
        goertzelAmplitude(samples, sampleRate, targetHz, channel, channels),
      );
  }
  return {
    sampleRate,
    channels,
    frames: samples.length / channels,
    durationSeconds: samples.length / channels / sampleRate,
    rms,
    peak,
    nonzeroRatio: nonzeroSamples / samples.length,
    targetHz: targetHz ?? null,
    targetAmplitude,
    toneToRmsRatio: targetAmplitude == null || rms === 0 ? null : targetAmplitude / rms,
  };
}

export function assertRecordedWaveform(
  result,
  {
    minimumRms = 1e-4,
    minimumPeak = 1e-3,
    minimumNonzeroRatio = 0.001,
    minimumToneAmplitude = 1e-3,
  } = {},
) {
  const failures = [];
  if (result.rms < minimumRms) failures.push(`RMS ${result.rms} < ${minimumRms}`);
  if (result.peak < minimumPeak) failures.push(`peak ${result.peak} < ${minimumPeak}`);
  if (result.nonzeroRatio < minimumNonzeroRatio)
    failures.push(`nonzero ratio ${result.nonzeroRatio} < ${minimumNonzeroRatio}`);
  if (result.targetHz != null && result.targetAmplitude < minimumToneAmplitude)
    failures.push(
      `${result.targetHz} Hz amplitude ${result.targetAmplitude} < ${minimumToneAmplitude}`,
    );
  if (failures.length) throw new Error(`Recorded waveform gate failed: ${failures.join('; ')}`);
  return result;
}

async function decodeRecording({ ffmpeg, input, start, duration, audioStream }) {
  const args = [
    '-v',
    'error',
    '-ss',
    String(start),
    '-t',
    String(duration),
    '-i',
    input,
    '-map',
    `0:a:${audioStream}`,
    '-f',
    'f32le',
    '-acodec',
    'pcm_f32le',
    '-ar',
    String(DEFAULT_SAMPLE_RATE),
    '-ac',
    String(DEFAULT_CHANNELS),
    'pipe:1',
  ];
  const child = spawn(ffmpeg, args, { windowsHide: true, stdio: ['ignore', 'pipe', 'pipe'] });
  const output = [];
  const errors = [];
  child.stdout.on('data', (chunk) => output.push(chunk));
  child.stderr.on('data', (chunk) => errors.push(chunk));
  const exitCode = await new Promise((resolveExit, reject) => {
    child.once('error', reject);
    child.once('close', resolveExit);
  });
  if (exitCode !== 0)
    throw new Error(
      `FFmpeg decode failed (${exitCode}): ${Buffer.concat(errors).toString('utf8')}`,
    );
  const bytes = Buffer.concat(output);
  if (bytes.length === 0 || bytes.length % 4 !== 0)
    throw new Error(`FFmpeg returned invalid Float32 PCM (${bytes.length} bytes)`);
  return new Float32Array(bytes.buffer, bytes.byteOffset, bytes.byteLength / 4);
}

function parseArguments(arguments_) {
  const options = {
    ffmpeg: process.env.FFMPEG_PATH || 'ffmpeg',
    start: 0,
    duration: 8,
    audioStream: 0,
  };
  for (let index = 0; index < arguments_.length; index += 1) {
    const argument = arguments_[index];
    if (!argument.startsWith('--') && !options.input) options.input = argument;
    else if (argument === '--ffmpeg') options.ffmpeg = arguments_[++index];
    else if (argument === '--start') options.start = Number(arguments_[++index]);
    else if (argument === '--duration') options.duration = Number(arguments_[++index]);
    else if (argument === '--audio-stream') options.audioStream = Number(arguments_[++index]);
    else if (argument === '--expect-tone') options.targetHz = Number(arguments_[++index]);
    else if (argument === '--output') options.output = arguments_[++index];
    else throw new Error(`Unknown argument: ${argument}`);
  }
  if (!options.input) throw new Error('Usage: verify-recorded-waveform.mjs <recording> [options]');
  if (!(options.duration >= 5 && options.duration <= 10))
    throw new Error('--duration must be between 5 and 10 seconds');
  return options;
}

async function main() {
  const options = parseArguments(process.argv.slice(2));
  const samples = await decodeRecording(options);
  const result = assertRecordedWaveform(
    analyzePcmFloat32(samples, {
      sampleRate: DEFAULT_SAMPLE_RATE,
      channels: DEFAULT_CHANNELS,
      targetHz: options.targetHz,
    }),
  );
  const report = `${JSON.stringify({ input: resolve(options.input), ...result }, null, 2)}\n`;
  if (options.output) await writeFile(options.output, report);
  process.stdout.write(report);
}

if (process.argv[1] && resolve(process.argv[1]) === fileURLToPath(import.meta.url))
  main().catch((error) => {
    console.error(error.message);
    process.exitCode = 1;
  });
