import { describe, expect, it } from 'vitest';
import { EMPTY_TOTALS, envelope, type ChartFingerprint, type ClientHello } from '@bbt/protocol';
import { LobbyService } from '../src/lobby-service.js';
import { MemoryStore } from '../src/memory-store.js';
import type { UserRecord } from '../src/models.js';

const host: UserRecord = {
  id: 'host',
  displayName: 'Host',
  role: 'organizer',
  disabled: false,
  createdAtMs: 1,
};
const player: UserRecord = {
  id: 'player',
  displayName: 'Player',
  role: 'player',
  disabled: false,
  createdAtMs: 1,
};
const chart: ChartFingerprint = {
  algorithm: 'sha256-canonical-package-v1',
  hash: 'a'.repeat(64),
  packageName: 'chart.zip',
  songName: 'Signal',
  variant: 'Hard',
  expectedMaxHits: 100,
};
const client: ClientHello = {
  clientVersion: '0.1.0-alpha.1',
  gameBuildHash: 'supported-build',
  distribution: 'standalone',
  mods: [],
};

describe('lobby lifecycle and reconciliation', () => {
  it('requires chart and client compatibility before scheduling', async () => {
    const service = new LobbyService(new MemoryStore(), ['supported-build']);
    let lobby = await service.create(host, 'Finals');
    lobby = await service.join(lobby.code, player);
    lobby = await service.setChart(lobby.id, host, chart);
    await expect(service.setReady(lobby.id, host.id, true, chart.hash)).rejects.toThrow(
      'compatibility proof',
    );
    await expect(
      service.setReady(lobby.id, host.id, true, chart.hash, {
        ...client,
        gameBuildHash: 'unknown',
      }),
    ).rejects.toThrow('not supported');
    await service.setReady(lobby.id, host.id, true, chart.hash, client);
    lobby = await service.setReady(lobby.id, player.id, true, chart.hash, client);
    expect(lobby.lifecycle).toBe('ready');
    expect((await service.scheduleStart(lobby.id, host)).lifecycle).toBe('countdown');
  });

  it('deduplicates and recovers a temporary run sequence gap', async () => {
    const store = new MemoryStore();
    const service = new LobbyService(store, ['supported-build']);
    let lobby = await service.create(host, 'Recovery');
    lobby = await service.join(lobby.code, player);
    lobby = await service.setChart(lobby.id, host, chart);
    const score = (runSequence: number, currentMaxHits: number) =>
      envelope('run.score_delta', runSequence + 20, {
        lobbyId: lobby.id,
        runId: 'run-1',
        runSequence,
        progress: currentMaxHits / 100,
        beat: currentMaxHits,
        songTimeMs: currentMaxHits * 100,
        totals: {
          ...EMPTY_TOTALS,
          hits: currentMaxHits,
          combo: currentMaxHits,
          maxCombo: currentMaxHits,
          currentMaxHits,
          maxHits: 100,
        },
      });
    await service.ingest(player, score(0, 1));
    expect(
      (await service.ingest(player, score(2, 3))).lobby.players.find(
        (value) => value.userId === player.id,
      )?.validity,
    ).toBe('invalid');
    const recovered = await service.ingest(player, score(1, 2));
    expect(recovered.lobby.players.find((value) => value.userId === player.id)?.validity).toBe(
      'valid',
    );
    expect((await service.ingest(player, score(2, 3))).duplicate).toBe(true);
  });

  it('verifies the chart loaded by the mod and allows non-organizers to leave', async () => {
    const store = new MemoryStore();
    const service = new LobbyService(store, ['supported-build']);
    let lobby = await service.create(host, 'Installed mod flow');
    lobby = await service.join(lobby.code, player);
    lobby = await service.setChart(lobby.id, host, chart);
    const valid = await service.beginRun(player, lobby.id, 'run-valid', 100, chart.hash, 'Hard');
    expect(valid.players.find((value) => value.userId === player.id)?.validity).toBe('pending');
    const invalid = await service.beginRun(host, lobby.id, 'run-invalid', 99, chart.hash, 'Hard');
    expect(invalid.players.find((value) => value.userId === host.id)?.validity).toBe('invalid');
    const afterLeave = await service.leave(lobby.id, player);
    expect(afterLeave.players.some((value) => value.userId === player.id)).toBe(false);
    await expect(service.leave(lobby.id, host)).rejects.toThrow('organizer must close');
  });
});
