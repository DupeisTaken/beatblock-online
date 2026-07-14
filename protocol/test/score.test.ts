import { describe, expect, it } from 'vitest';
import { calculateAccuracy, EMPTY_TOTALS, rankPlayers } from '../src/index.js';

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
      userId: String(index),
      displayName,
      connected: true,
      ready: true,
      spectator: false,
      progress: 0.5,
      accuracy: 99,
      totals: { ...EMPTY_TOTALS, maxCombo: 10 },
      validity: 'valid' as const,
    }));
    expect(rankPlayers(players).find((player) => player.displayName === 'A')?.rank).toBe(1);
  });
});
