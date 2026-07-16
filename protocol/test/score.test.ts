import { describe, expect, it } from 'vitest';
import { calculateAccuracy, decodeRenderDatagram, EMPTY_TOTALS, encodeRenderDatagram, rankPlayers } from '../src/index.js';

describe('Beatblock score derivation', () => {
  it('matches perfect, barely, and miss scoring', () => {
    expect(calculateAccuracy({ currentMaxHits: 100, misses: 0, barelies: 0 })).toBe(100);
    expect(calculateAccuracy({ currentMaxHits: 100, misses: 0, barelies: 1 })).toBe(99.75);
    expect(calculateAccuracy({ currentMaxHits: 100, misses: 1, barelies: 0 })).toBe(99);
  });

  it('floors to two decimal places', () => {
    expect(calculateAccuracy({ currentMaxHits: 3, misses: 1, barelies: 0 })).toBe(66.66);
  });

  it('ranks by accuracy, progress, max combo, then name', () => {
    const players = ['B', 'A'].map((displayName, index) => ({
      sessionId: String(index),
      displayName,
      role: 'player' as const,
      admitted: true,
      connected: true,
      ready: true,
      verified: true,
      progress: 0.5,
      accuracy: 99,
      setTotal: 0,
      totals: { ...EMPTY_TOTALS, maxCombo: 10 },
      validity: 'valid' as const,
    }));
    expect(rankPlayers(players).find((player) => player.displayName === 'A')?.rank).toBe(1);
  });

  it('round-trips the packed 60 Hz renderer datagram', () => {
    const encoded = encodeRenderDatagram({ sessionId: 2, sequence: 418, runTimeUs: 82431000n, beat: 42.5, paddleAngle: 187.25, tapMask: 3, flags: 1 });
    expect(encoded).toHaveLength(32);
    expect(decodeRenderDatagram(encoded)).toEqual({ sessionId: 2, sequence: 418, runTimeUs: 82431000n, beat: 42.5, paddleAngle: 187.25, tapMask: 3, flags: 1 });
  });
});
