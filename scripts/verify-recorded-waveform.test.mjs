import assert from 'node:assert/strict';
import test from 'node:test';
import { analyzePcmFloat32, assertRecordedWaveform } from './verify-recorded-waveform.mjs';

function stereoTone({ frequency = 1_000, seconds = 5, amplitude = 0.25 } = {}) {
  const sampleRate = 48_000;
  const channels = 2;
  const samples = new Float32Array(sampleRate * seconds * channels);
  for (let frame = 0; frame < sampleRate * seconds; frame += 1) {
    const sample = amplitude * Math.sin((2 * Math.PI * frequency * frame) / sampleRate);
    samples[frame * channels] = sample;
    samples[frame * channels + 1] = sample;
  }
  return samples;
}

test('saved-track gate accepts a deterministic nonzero 1 kHz waveform', () => {
  const result = analyzePcmFloat32(stereoTone(), { targetHz: 1_000 });
  assert.ok(result.rms > 0.17 && result.rms < 0.18);
  assert.ok(result.peak > 0.249);
  assert.ok(result.targetAmplitude > 0.249);
  assert.doesNotThrow(() => assertRecordedWaveform(result));
});

test('saved-track gate rejects digital silence even when a stream exists', () => {
  const result = analyzePcmFloat32(new Float32Array(48_000 * 5 * 2), { targetHz: 1_000 });
  assert.throws(
    () => assertRecordedWaveform(result),
    /RMS.*peak.*nonzero ratio.*1000 Hz amplitude/,
  );
});

test('tone gate rejects nonzero PCM that lacks the expected frequency', () => {
  const result = analyzePcmFloat32(stereoTone({ frequency: 440 }), { targetHz: 1_000 });
  assert.ok(result.rms > 0.1);
  assert.throws(() => assertRecordedWaveform(result), /1000 Hz amplitude/);
});
