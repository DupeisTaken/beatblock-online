import { mkdir, writeFile } from 'node:fs/promises';
import { monitorEventLoopDelay, performance } from 'node:perf_hooks';
import { fileURLToPath } from 'node:url';
import { EMPTY_TOTALS, envelope, type ChartFingerprint, type ClientHello } from '@bbt/protocol';
import { AuthService } from '../src/auth-service.js';
import type { Config } from '../src/config.js';
import { LobbyError, LobbyService } from '../src/lobby-service.js';
import { MemoryStore } from '../src/memory-store.js';
import type { UserRecord } from '../src/models.js';

const build = 'benchmark-supported-build';
const config: Config = {
  host: '127.0.0.1',
  port: 0,
  publicUrl: 'http://127.0.0.1',
  databaseUrl: 'memory://',
  tokenSecret: 'benchmark-token-secret-that-is-not-for-production',
  inviteSecret: 'benchmark-invite-secret-that-is-not-for-production',
  accessTokenTtlSeconds: 900,
  refreshTokenTtlDays: 30,
  runEventRetentionDays: 30,
  allowInsecureHttp: true,
  supportedGameBuilds: [build],
};
const client: ClientHello = {
  clientVersion: '0.1.0-alpha.1',
  gameBuildHash: build,
  distribution: 'standalone',
  mods: [],
};
const chart: ChartFingerprint = {
  algorithm: 'sha256-canonical-package-v1',
  hash: 'c'.repeat(64),
  packageName: 'maximum-grid.zip',
  songName: 'Stress Signal',
  variant: 'Competition',
  expectedMaxHits: 240,
};

function directUser(index: number, spectator = false): UserRecord {
  return {
    id: `${spectator ? 'spectator' : 'player'}-${index}`,
    displayName: `${spectator ? 'Spectator' : 'Player'} ${index}`,
    role: index === 0 && !spectator ? 'organizer' : 'player',
    disabled: false,
    createdAtMs: index + 1,
  };
}

function percentile(values: number[], fraction: number): number {
  const sorted = [...values].sort((left, right) => left - right);
  return sorted[Math.round((sorted.length - 1) * fraction)] ?? 0;
}

async function main() {
  const store = new MemoryStore();
  const service = new LobbyService(store, [build]);
  const auth = new AuthService(store, config);
  const loopDelay = monitorEventLoopDelay({ resolution: 10 });
  loopDelay.enable();

  const invite = await auth.createInvite('player', 1, Date.now() + 60_000);
  const redeemed = await auth.redeem(invite.code, 'Invited Benchmark User', 'Trial Runner');
  const refreshed = await auth.refresh(redeemed.refreshToken);
  const ticket = await auth.createBrowserTicket(redeemed.user.id);
  const browser = await auth.exchangeBrowserTicket(ticket);
  let ticketReuseRejected = false;
  try {
    await auth.exchangeBrowserTicket(ticket);
  } catch {
    ticketReuseRejected = true;
  }

  const players = Array.from({ length: 16 }, (_, index) => directUser(index));
  const spectators = Array.from({ length: 32 }, (_, index) => directUser(index, true));
  let lobby = await service.create(players[0]!, 'Maximum Grid');
  const setupStarted = performance.now();
  await Promise.all(players.slice(1).map((user) => service.join(lobby.code, user)));
  await Promise.all(spectators.map((user) => service.join(lobby.code, user, true)));
  const setupMs = performance.now() - setupStarted;

  let playerCapacityRejected = false;
  let spectatorCapacityRejected = false;
  try {
    await service.join(lobby.code, directUser(16));
  } catch (error) {
    playerCapacityRejected = error instanceof LobbyError && error.statusCode === 409;
  }
  try {
    await service.join(lobby.code, directUser(32, true), true);
  } catch (error) {
    spectatorCapacityRejected = error instanceof LobbyError && error.statusCode === 409;
  }

  await service.setChart(lobby.id, players[0]!, chart);
  await Promise.all(
    players.map((user) => service.setReady(lobby.id, user.id, true, chart.hash, client)),
  );
  lobby = await service.scheduleStart(lobby.id, players[0]!);

  const operationMs: number[] = [];
  const ingestStarted = performance.now();
  for (let sequence = 0; sequence < 240; sequence += 1) {
    await Promise.all(
      players.map(async (user, playerIndex) => {
        const started = performance.now();
        await service.ingest(
          user,
          envelope('run.score_delta', sequence, {
            lobbyId: lobby.id,
            runId: `run-${playerIndex}`,
            runSequence: sequence,
            progress: (sequence + 1) / 240,
            beat: sequence,
            songTimeMs: sequence * 100,
            totals: {
              ...EMPTY_TOTALS,
              hits: sequence + 1,
              combo: sequence + 1,
              maxCombo: sequence + 1,
              currentMaxHits: sequence + 1,
              maxHits: 240,
            },
          }),
        );
        operationMs.push(performance.now() - started);
      }),
    );
  }
  const ingestSeconds = (performance.now() - ingestStarted) / 1_000;

  const beforeDuplicate = (await store.getStatus()).runEvents;
  await service.ingest(
    players[15]!,
    envelope('run.score_delta', 239, {
      lobbyId: lobby.id,
      runId: 'run-15',
      runSequence: 239,
      progress: 1,
      beat: 239,
      songTimeMs: 23_900,
      totals: {
        ...EMPTY_TOTALS,
        hits: 240,
        combo: 240,
        maxCombo: 240,
        currentMaxHits: 240,
        maxHits: 240,
      },
    }),
  );
  const duplicateIdempotent = (await store.getStatus()).runEvents === beforeDuplicate;

  await service.disconnect(players[15]!.id);
  await service.join(lobby.id, players[15]!);
  const reconnectAtCapacity = (await store.getLobby(lobby.id))!.players.find(
    (value) => value.userId === players[15]!.id,
  )!.connected;
  await Promise.all(players.map((user) => service.finish(lobby.id, user.id)));
  lobby = (await store.getLobby(lobby.id))!;
  loopDelay.disable();

  const events = (await store.getStatus()).runEvents;
  const throughput = events / ingestSeconds;
  const passed =
    events === 16 * 240 &&
    lobby.lifecycle === 'results' &&
    playerCapacityRejected &&
    spectatorCapacityRejected &&
    duplicateIdempotent &&
    reconnectAtCapacity &&
    ticketReuseRejected &&
    operationMs.every(Number.isFinite) &&
    percentile(operationMs, 0.95) < 250;
  const report = {
    schemaVersion: 1,
    generatedAt: new Date().toISOString(),
    passed,
    capabilities: {
      inviteRedemption: Boolean(redeemed.accessToken),
      rotatingRefresh: refreshed.refreshToken !== redeemed.refreshToken,
      oneTimeBrowserHandoff: Boolean(browser.accessToken) && ticketReuseRejected,
      maximumLobby: lobby.players.length === 48,
      playerCapacityRejected,
      spectatorCapacityRejected,
      chartVerificationAndReadyCheck: Boolean(lobby.chart),
      synchronizedStartScheduled: Boolean(lobby.scheduledStartTimeMs),
      authoritativeResults: lobby.lifecycle === 'results',
      duplicateIdempotent,
      reconnectAtCapacity,
    },
    workload: { players: 16, spectators: 32, eventsPerPlayer: 240, totalEvents: events },
    metrics: {
      concurrentJoinMs: setupMs,
      scoreEventsPerSecond: throughput,
      ingestMeanMs: operationMs.reduce((sum, value) => sum + value, 0) / operationMs.length,
      ingestP95Ms: percentile(operationMs, 0.95),
      ingestP99Ms: percentile(operationMs, 0.99),
      eventLoopDelayP95Ms: loopDelay.percentile(95) / 1_000_000,
      heapUsedMb: process.memoryUsage().heapUsed / 1024 / 1024,
    },
    thresholds: { exactEventCount: 3_840, ingestP95Ms: 250 },
  };
  const reportDirectory = fileURLToPath(new URL('../../reports/trial-runs/', import.meta.url));
  await mkdir(reportDirectory, { recursive: true });
  await writeFile(
    `${reportDirectory}/server-stress-latest.json`,
    `${JSON.stringify(report, null, 2)}\n`,
  );
  console.log(JSON.stringify(report, null, 2));
  if (!passed) throw new Error('server stress benchmark thresholds were not met');
}

await main();
