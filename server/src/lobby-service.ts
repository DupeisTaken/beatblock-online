import {
  calculateAccuracy,
  DEFAULT_RULES,
  EMPTY_TOTALS,
  MAX_PLAYERS,
  MAX_SPECTATORS,
  rankPlayers,
  type ChartFingerprint,
  type Envelope,
  type LobbySnapshot,
  type RunScoreDelta,
  type ClientHello,
} from '@bbt/protocol';
import type { Store, UserRecord } from './models.js';
import { createLobbyCode, id } from './security.js';

export class LobbyError extends Error {
  constructor(
    message: string,
    public statusCode = 400,
  ) {
    super(message);
  }
}

export class LobbyService {
  private lobbyLocks = new Map<string, Promise<void>>();

  constructor(
    private store: Store,
    private supportedGameBuilds: string[],
  ) {}

  async create(organizer: UserRecord, name: string): Promise<LobbySnapshot> {
    if (organizer.role !== 'organizer' && organizer.role !== 'operator')
      throw new LobbyError('Only organizers may create lobbies', 403);
    const now = Date.now();
    const lobby: LobbySnapshot = {
      id: id(),
      code: createLobbyCode(),
      name: name.trim().slice(0, 80) || `${organizer.displayName}'s lobby`,
      organizerId: organizer.id,
      lifecycle: 'forming',
      rules: { ...DEFAULT_RULES },
      players: [this.player(organizer, false)],
      createdAtMs: now,
      updatedAtMs: now,
    };
    await this.store.saveLobby(lobby);
    return lobby;
  }

  async join(code: string, user: UserRecord, spectator = false): Promise<LobbySnapshot> {
    return this.withLobbyLock(code, async (lobby) => {
      if (lobby.lifecycle === 'closed') throw new LobbyError('Lobby is closed', 409);
      const existing = lobby.players.find((item) => item.userId === user.id);
      if (existing) {
        existing.connected = true;
        existing.spectator = spectator;
        lobby.updatedAtMs = Date.now();
        await this.store.saveLobby(lobby);
        return lobby;
      }
      const competitors = lobby.players.filter((item) => !item.spectator);
      const spectators = lobby.players.filter((item) => item.spectator);
      if (!spectator && competitors.length >= MAX_PLAYERS)
        throw new LobbyError('Lobby is full', 409);
      if (spectator && spectators.length >= MAX_SPECTATORS)
        throw new LobbyError('Spectator capacity reached', 409);
      lobby.players.push(this.player(user, spectator));
      lobby.updatedAtMs = Date.now();
      await this.store.saveLobby(lobby);
      return lobby;
    });
  }

  async setChart(
    lobbyId: string,
    actor: UserRecord,
    chart: ChartFingerprint,
  ): Promise<LobbySnapshot> {
    return this.withLobbyLock(lobbyId, async (lobby) => {
      this.requireOrganizer(lobby, actor);
      if (!['forming', 'chart_locked'].includes(lobby.lifecycle))
        throw new LobbyError('Chart cannot be changed after countdown', 409);
      lobby.chart = chart;
      lobby.lifecycle = 'chart_locked';
      lobby.players = lobby.players.map((player) => ({ ...player, ready: player.spectator }));
      lobby.updatedAtMs = Date.now();
      await this.store.saveLobby(lobby);
      return lobby;
    });
  }

  async setReady(
    lobbyId: string,
    userId: string,
    ready: boolean,
    fingerprint?: string,
    client?: ClientHello,
  ): Promise<LobbySnapshot> {
    return this.withLobbyLock(lobbyId, async (lobby) => {
      if (!lobby.chart) throw new LobbyError('Organizer has not locked a chart', 409);
      const player = lobby.players.find((item) => item.userId === userId);
      if (!player) throw new LobbyError('User is not in this lobby', 404);
      if (!player.spectator && ready && fingerprint !== lobby.chart.hash)
        throw new LobbyError('Chart package does not match the lobby', 409);
      if (!player.spectator && ready) await this.requireCompatibleClient(client);
      player.ready = ready;
      player.connected = true;
      const competitors = lobby.players.filter((item) => !item.spectator);
      lobby.lifecycle =
        competitors.length >= 2 && competitors.every((item) => item.ready)
          ? 'ready'
          : 'chart_locked';
      lobby.updatedAtMs = Date.now();
      await this.store.saveLobby(lobby);
      return lobby;
    });
  }

  async scheduleStart(lobbyId: string, actor: UserRecord): Promise<LobbySnapshot> {
    return this.withLobbyLock(lobbyId, async (lobby) => {
      this.requireOrganizer(lobby, actor);
      if (lobby.lifecycle !== 'ready') throw new LobbyError('Every competitor must be ready', 409);
      lobby.lifecycle = 'countdown';
      lobby.scheduledStartTimeMs = Date.now() + 5_000;
      lobby.updatedAtMs = Date.now();
      await this.store.saveLobby(lobby);
      return lobby;
    });
  }

  async ingest(
    user: UserRecord,
    envelope: Envelope<RunScoreDelta>,
  ): Promise<{ lobby: LobbySnapshot; duplicate: boolean }> {
    const payload = envelope.payload;
    return this.withLobbyLock(payload.lobbyId, async (lobby) => {
      const player = lobby.players.find((item) => item.userId === user.id && !item.spectator);
      if (!player) throw new LobbyError('Run user is not a competitor', 403);
      const previousSequence = await this.store.getRunSequenceState(payload.runId);
      const outcome = await this.store.appendRunEvent({
        lobbyId: lobby.id,
        runId: payload.runId,
        userId: user.id,
        sequence: payload.runSequence,
        receivedAtMs: Date.now(),
        envelope,
      });
      if (outcome === 'duplicate') return { lobby, duplicate: true };
      if (
        (!previousSequence && payload.runSequence !== 0) ||
        (previousSequence && payload.runSequence > previousSequence.max + 1)
      ) {
        player.validity = 'invalid';
        player.invalidReason = `Run event sequence gap before ${payload.runSequence}`;
      }
      if (
        lobby.lifecycle === 'countdown' &&
        lobby.scheduledStartTimeMs &&
        Date.now() >= lobby.scheduledStartTimeMs - 250
      )
        lobby.lifecycle = 'playing';
      const lateRecoveryEvent =
        previousSequence !== undefined && payload.runSequence <= previousSequence.max;
      if (!lateRecoveryEvent) {
        if (!this.validTotals(player.totals, payload.totals)) {
          player.validity = 'invalid';
          player.invalidReason = 'Non-monotonic or inconsistent score totals';
        } else {
          player.totals = payload.totals;
          player.progress = payload.progress;
          player.accuracy = calculateAccuracy(payload.totals);
          if (player.validity === 'pending') player.validity = 'valid';
        }
      }
      const sequenceState = await this.store.getRunSequenceState(payload.runId);
      if (
        sequenceState &&
        sequenceState.min === 0 &&
        sequenceState.count === sequenceState.max + 1 &&
        player.invalidReason?.startsWith('Run event sequence gap')
      ) {
        player.validity = 'valid';
        delete player.invalidReason;
      }
      lobby.players = rankPlayers(lobby.players);
      lobby.updatedAtMs = Date.now();
      await this.store.saveRunSummary({
        runId: payload.runId,
        lobbyId: lobby.id,
        userId: user.id,
        accuracy: player.accuracy,
        progress: player.progress,
        validity: player.validity,
        ...(player.invalidReason ? { invalidReason: player.invalidReason } : {}),
        totals: player.totals,
        updatedAtMs: lobby.updatedAtMs,
      });
      await this.store.saveLobby(lobby);
      return { lobby, duplicate: false };
    });
  }

  async beginRun(
    user: UserRecord,
    lobbyId: string,
    runId: string,
    maxHits: number,
    chartHash?: string,
    variant?: string,
  ): Promise<LobbySnapshot> {
    return this.withLobbyLock(lobbyId, async (lobby) => {
      const player = lobby.players.find((item) => item.userId === user.id && !item.spectator);
      if (!player) throw new LobbyError('Run user is not a competitor', 403);
      if (!lobby.chart) throw new LobbyError('Lobby chart is unavailable', 409);
      if (
        chartHash !== lobby.chart.hash ||
        variant !== lobby.chart.variant ||
        maxHits !== lobby.chart.expectedMaxHits
      ) {
        player.validity = 'invalid';
        player.invalidReason =
          chartHash !== lobby.chart.hash
            ? 'Loaded chart package does not match the lobby'
            : variant !== lobby.chart.variant
              ? `Expected variant ${lobby.chart.variant} but loaded ${variant ?? 'unknown'}`
              : `Expected ${lobby.chart.expectedMaxHits} score hits but loaded ${maxHits}`;
      }
      if (
        lobby.lifecycle === 'countdown' &&
        lobby.scheduledStartTimeMs &&
        Date.now() >= lobby.scheduledStartTimeMs - 250
      )
        lobby.lifecycle = 'playing';
      lobby.updatedAtMs = Date.now();
      await this.store.saveRunSummary({
        runId,
        lobbyId,
        userId: user.id,
        accuracy: player.accuracy,
        progress: player.progress,
        validity: player.validity,
        ...(player.invalidReason ? { invalidReason: player.invalidReason } : {}),
        totals: player.totals,
        updatedAtMs: lobby.updatedAtMs,
      });
      await this.store.saveLobby(lobby);
      return lobby;
    });
  }

  async invalidate(
    lobbyId: string,
    userId: string,
    reason: string,
    dnf = false,
    runId?: string,
  ): Promise<LobbySnapshot> {
    return this.withLobbyLock(lobbyId, async (lobby) => {
      const player = lobby.players.find((item) => item.userId === userId);
      if (!player) throw new LobbyError('Run user not found', 404);
      player.validity = dnf ? 'dnf' : 'invalid';
      player.invalidReason = reason.slice(0, 256);
      player.ready = false;
      lobby.players = rankPlayers(lobby.players);
      lobby.updatedAtMs = Date.now();
      if (runId)
        await this.store.saveRunSummary({
          runId,
          lobbyId: lobby.id,
          userId,
          accuracy: player.accuracy,
          progress: player.progress,
          validity: player.validity,
          invalidReason: player.invalidReason,
          totals: player.totals,
          updatedAtMs: lobby.updatedAtMs,
        });
      await this.store.saveLobby(lobby);
      return lobby;
    });
  }

  async finish(lobbyId: string, userId: string): Promise<LobbySnapshot> {
    return this.withLobbyLock(lobbyId, async (lobby) => {
      const player = lobby.players.find((item) => item.userId === userId);
      if (!player) throw new LobbyError('Run user not found', 404);
      player.progress = 1;
      const competitors = lobby.players.filter((item) => !item.spectator);
      if (
        competitors.every(
          (item) => item.progress >= 1 || ['invalid', 'dnf'].includes(item.validity),
        )
      )
        lobby.lifecycle = 'results';
      lobby.players = rankPlayers(lobby.players);
      lobby.updatedAtMs = Date.now();
      await this.store.saveLobby(lobby);
      return lobby;
    });
  }

  async disconnect(userId: string): Promise<LobbySnapshot[]> {
    const changed: LobbySnapshot[] = [];
    for (const snapshot of await this.store.listLobbies()) {
      const lobby = await this.withLobbyLock(snapshot.id, async (current) => {
        const player = current.players.find((item) => item.userId === userId);
        if (!player || !player.connected || ['results', 'closed'].includes(current.lifecycle))
          return undefined;
        player.connected = false;
        current.updatedAtMs = Date.now();
        await this.store.saveLobby(current);
        return current;
      });
      if (lobby) changed.push(lobby);
    }
    return changed;
  }

  async leave(lobbyId: string, user: UserRecord): Promise<LobbySnapshot> {
    return this.withLobbyLock(lobbyId, async (lobby) => {
      if (lobby.organizerId === user.id)
        throw new LobbyError('The organizer must close the lobby', 409);
      const before = lobby.players.length;
      lobby.players = lobby.players.filter((player) => player.userId !== user.id);
      if (lobby.players.length === before) throw new LobbyError('User is not in this lobby', 404);
      lobby.updatedAtMs = Date.now();
      await this.store.saveLobby(lobby);
      return lobby;
    });
  }

  async close(lobbyId: string, actor: UserRecord): Promise<LobbySnapshot> {
    return this.withLobbyLock(lobbyId, async (lobby) => {
      this.requireOrganizer(lobby, actor);
      lobby.lifecycle = 'closed';
      lobby.updatedAtMs = Date.now();
      await this.store.saveLobby(lobby);
      return lobby;
    });
  }

  private player(user: UserRecord, spectator: boolean) {
    return {
      userId: user.id,
      displayName: user.displayName,
      connected: true,
      ready: spectator,
      spectator,
      progress: 0,
      accuracy: 100,
      totals: { ...EMPTY_TOTALS },
      validity: 'pending' as const,
    };
  }
  private async requireLobby(value: string): Promise<LobbySnapshot> {
    const lobby = await this.store.getLobby(value);
    if (!lobby) throw new LobbyError('Lobby not found', 404);
    return lobby;
  }
  private async withLobbyLock<T>(
    idOrCode: string,
    operation: (lobby: LobbySnapshot) => Promise<T>,
  ): Promise<T> {
    const resolved = await this.requireLobby(idOrCode);
    const previous = this.lobbyLocks.get(resolved.id) ?? Promise.resolve();
    let release!: () => void;
    const current = new Promise<void>((resolve) => {
      release = resolve;
    });
    const tail = previous.then(() => current);
    this.lobbyLocks.set(resolved.id, tail);
    await previous;
    try {
      return await operation(await this.requireLobby(resolved.id));
    } finally {
      release();
      if (this.lobbyLocks.get(resolved.id) === tail) this.lobbyLocks.delete(resolved.id);
    }
  }
  private requireOrganizer(lobby: LobbySnapshot, actor: UserRecord): void {
    if (lobby.organizerId !== actor.id && actor.role !== 'operator')
      throw new LobbyError('Organizer permission required', 403);
  }
  private async requireCompatibleClient(client?: ClientHello): Promise<void> {
    if (!client) throw new LobbyError('Client compatibility proof is required', 409);
    if (!/^0\.1\./.test(client.clientVersion))
      throw new LobbyError(
        `Client version ${client.clientVersion} is incompatible with this alpha`,
        409,
      );
    if (!this.supportedGameBuilds.includes(client.gameBuildHash.toLowerCase()))
      throw new LobbyError('This Beatblock build is not supported for competitive play', 409);
    const allowed = new Map((await this.store.listAllowedMods()).map((mod) => [mod.id, mod.hash]));
    const unapproved = client.mods.find((mod) => allowed.get(mod.id) !== mod.hash);
    if (unapproved)
      throw new LobbyError(`Mod ${unapproved.id} is not on the instance allowlist`, 409);
  }
  private validTotals(previous: typeof EMPTY_TOTALS, next: typeof EMPTY_TOTALS): boolean {
    return (
      next.hits >= previous.hits &&
      next.misses >= previous.misses &&
      next.barelies >= previous.barelies &&
      next.currentMaxHits >= previous.currentMaxHits &&
      next.maxHits >= next.currentMaxHits &&
      next.combo <= next.maxCombo &&
      next.maxCombo >= previous.maxCombo
    );
  }
}
