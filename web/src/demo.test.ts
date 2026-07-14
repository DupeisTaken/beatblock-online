import { describe, expect, it } from 'vitest';
import { demoLobby } from './demo';

describe('broadcast demo fixture', () => {
  it('contains ranked competitors and a valid chart fingerprint', () => {
    expect(demoLobby.players.filter((player) => !player.spectator)).toHaveLength(5);
    expect(demoLobby.players.find((player) => player.rank === 1)?.displayName).toBe('MiraFlux');
    expect(demoLobby.chart?.hash).toMatch(/^[a-f0-9]{64}$/);
  });
});
