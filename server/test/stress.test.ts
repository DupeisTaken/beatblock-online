import { describe, expect, it } from 'vitest';
import { EMPTY_TOTALS, envelope, type ChartFingerprint, type ClientHello } from '@bbt/protocol';
import { LobbyService } from '../src/lobby-service.js';
import { MemoryStore } from '../src/memory-store.js';
import type { UserRecord } from '../src/models.js';

const build = 'stress-supported-build';
const client: ClientHello = {
  clientVersion: '0.1.0-alpha.1',
  gameBuildHash: build,
  distribution: 'standalone',
  mods: [],
};
const chart: ChartFingerprint = {
  algorithm: 'sha256-canonical-package-v1',
  hash: 'b'.repeat(64),
  packageName: 'stress.zip',
  songName: 'Stress Signal',
  variant: 'Competition',
  expectedMaxHits: 120,
};

function user(index: number, spectator = false): UserRecord {
  return {
    id: `${spectator ? 'spectator' : 'player'}-${index}`,
    displayName: `${spectator ? 'Spectator' : 'Player'} ${index}`,
    role: index === 0 && !spectator ? 'organizer' : 'player',
    disabled: false,
    createdAtMs: index + 1,
  };
}

describe('maximum alpha lobby stress', () => {
  it('preserves 16 concurrent competitors, 32 spectators, and 1,920 score events', async () => {
    const store = new MemoryStore();
    const service = new LobbyService(store, [build]);
    const players = Array.from({ length: 16 }, (_, index) => user(index));
    const spectators = Array.from({ length: 32 }, (_, index) => user(index, true));
    let lobby = await service.create(players[0]!, 'Maximum Grid');

    await Promise.all(players.slice(1).map((value) => service.join(lobby.code, value)));
    await Promise.all(spectators.map((value) => service.join(lobby.code, value, true)));
    lobby = (await store.getLobby(lobby.id))!;
    expect(lobby.players.filter((value) => !value.spectator)).toHaveLength(16);
    expect(lobby.players.filter((value) => value.spectator)).toHaveLength(32);

    await service.setChart(lobby.id, players[0]!, chart);
    await Promise.all(
      players.map((value) => service.setReady(lobby.id, value.id, true, chart.hash, client)),
    );
    lobby = (await store.getLobby(lobby.id))!;
    expect(lobby.lifecycle).toBe('ready');

    for (let sequence = 0; sequence < 120; sequence += 1) {
      await Promise.all(
        players.map((value, playerIndex) =>
          service.ingest(
            value,
            envelope('run.score_delta', sequence, {
              lobbyId: lobby.id,
              runId: `run-${playerIndex}`,
              runSequence: sequence,
              progress: (sequence + 1) / 120,
              beat: sequence,
              songTimeMs: sequence * 100,
              totals: {
                ...EMPTY_TOTALS,
                hits: sequence + 1,
                combo: sequence + 1,
                maxCombo: sequence + 1,
                currentMaxHits: sequence + 1,
                maxHits: 120,
              },
            }),
          ),
        ),
      );
    }

    lobby = (await store.getLobby(lobby.id))!;
    expect(
      lobby.players.filter((value) => !value.spectator).every((value) => value.progress === 1),
    ).toBe(true);
    expect((await store.getStatus()).runEvents).toBe(16 * 120);
    expect(
      lobby.players
        .filter((value) => !value.spectator)
        .map((value) => value.rank)
        .sort((left, right) => (left ?? 0) - (right ?? 0)),
    ).toEqual(Array.from({ length: 16 }, (_, index) => index + 1));
  }, 20_000);
});
