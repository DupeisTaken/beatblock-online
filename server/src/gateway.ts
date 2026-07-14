import type { Envelope, LobbySnapshot, RunScoreDelta } from '@bbt/protocol';
import { envelope as makeEnvelope, isEnvelope, isRunScoreDelta } from '@bbt/protocol';
import type { WebSocket } from 'ws';
import type { UserRecord } from './models.js';
import { LobbyService } from './lobby-service.js';

interface Connection {
  socket: WebSocket;
  user: UserRecord;
  lobbyIds: Set<string>;
}

export class GatewayHub {
  private connections = new Set<Connection>();
  private sequence = 0;
  private lastLeaderboardAt = new Map<string, number>();
  private pendingLeaderboards = new Map<
    string,
    { lobby: LobbySnapshot; timer: ReturnType<typeof setTimeout> }
  >();
  constructor(private lobbies: LobbyService) {}

  add(socket: WebSocket, user: UserRecord): Connection {
    const connection = { socket, user, lobbyIds: new Set<string>() };
    this.connections.add(connection);
    socket.on('close', () => {
      this.connections.delete(connection);
      void this.lobbies
        .disconnect(user.id)
        .then((lobbies) => lobbies.forEach((lobby) => this.broadcastLobby(lobby)));
    });
    return connection;
  }

  subscribe(connection: Connection, lobbyId: string): void {
    connection.lobbyIds.add(lobbyId);
  }

  async receive(connection: Connection, value: string): Promise<void> {
    let message: Envelope;
    try {
      message = JSON.parse(value) as Envelope;
    } catch {
      this.send(
        connection.socket,
        makeEnvelope('error', this.sequence++, {
          code: 'invalid_json',
          message: 'Message is not valid JSON',
        }),
      );
      return;
    }
    if (!isEnvelope(message)) {
      this.send(
        connection.socket,
        makeEnvelope('error', this.sequence++, {
          code: 'invalid_envelope',
          message: 'Unsupported or malformed protocol envelope',
        }),
      );
      return;
    }
    try {
      if (message.type === 'lobby.subscribe') {
        const payload = message.payload as { lobbyId?: string };
        if (!payload.lobbyId) throw new Error('lobbyId is required');
        this.subscribe(connection, payload.lobbyId);
        return;
      }
      if (message.type === 'lobby.unsubscribe') {
        const payload = message.payload as { lobbyId?: string };
        if (!payload.lobbyId) throw new Error('lobbyId is required');
        connection.lobbyIds.delete(payload.lobbyId);
        return;
      }
      if (message.type === 'clock.ping') {
        const clientSendTimeMs = Number(
          (message.payload as { clientSendTimeMs?: number }).clientSendTimeMs,
        );
        const serverReceiveTimeMs = Date.now();
        this.send(
          connection.socket,
          makeEnvelope('clock.pong', this.sequence++, {
            clientSendTimeMs,
            serverReceiveTimeMs,
            serverSendTimeMs: Date.now(),
          }),
        );
        return;
      }
      if (message.type === 'client.hello') return;
      if (message.type === 'run.started') {
        const payload = message.payload as {
          lobbyId?: string;
          runId?: string;
          maxHits?: number;
          chartHash?: string;
          variant?: string;
        };
        const lobbyId = String(payload.lobbyId ?? '');
        if (!lobbyId) throw new Error('lobbyId is required');
        this.subscribe(connection, lobbyId);
        const lobby = await this.lobbies.beginRun(
          connection.user,
          lobbyId,
          String(payload.runId ?? ''),
          Number(payload.maxHits ?? 0),
          payload.chartHash,
          payload.variant,
        );
        this.broadcastLobby(lobby);
        return;
      }
      if (message.type === 'run.score_delta') {
        if (!isRunScoreDelta(message.payload))
          throw new Error('run.score_delta payload does not conform to protocol v1');
        const outcome = await this.lobbies.ingest(
          connection.user,
          message as Envelope<RunScoreDelta>,
        );
        this.subscribe(connection, outcome.lobby.id);
        if (!outcome.duplicate) this.broadcastLeaderboard(outcome.lobby);
        return;
      }
      if (message.type === 'run.invalid') {
        const payload = message.payload as {
          lobbyId: string;
          runId?: string;
          reason: string;
          dnf?: boolean;
        };
        const lobby = await this.lobbies.invalidate(
          payload.lobbyId,
          connection.user.id,
          payload.reason,
          payload.dnf,
          payload.runId,
        );
        this.broadcastLobby(lobby);
        return;
      }
      if (message.type === 'run.finished') {
        const payload = message.payload as { lobbyId: string };
        const lobby = await this.lobbies.finish(payload.lobbyId, connection.user.id);
        this.broadcastLobby(lobby);
        return;
      }
      this.send(
        connection.socket,
        makeEnvelope('error', this.sequence++, {
          code: 'unknown_type',
          message: `Unknown message type: ${message.type}`,
        }),
      );
    } catch (error) {
      this.send(
        connection.socket,
        makeEnvelope('error', this.sequence++, {
          code: 'message_rejected',
          message: error instanceof Error ? error.message : 'Message rejected',
        }),
      );
    }
  }

  broadcastLobby(lobby: LobbySnapshot): void {
    this.lastLeaderboardAt.set(lobby.id, Date.now());
    const message = makeEnvelope('lobby.snapshot', this.sequence++, lobby);
    for (const connection of this.connections) {
      if (
        connection.lobbyIds.has(lobby.id) ||
        lobby.players.some((player) => player.userId === connection.user.id)
      )
        this.send(connection.socket, message);
    }
  }

  private broadcastLeaderboard(lobby: LobbySnapshot): void {
    const elapsed = Date.now() - (this.lastLeaderboardAt.get(lobby.id) ?? 0);
    if (elapsed >= 100) {
      this.broadcastLobby(lobby);
      return;
    }
    const pending = this.pendingLeaderboards.get(lobby.id);
    if (pending) {
      pending.lobby = lobby;
      return;
    }
    const holder = {
      lobby,
      timer: setTimeout(() => {
        this.pendingLeaderboards.delete(lobby.id);
        this.broadcastLobby(holder.lobby);
      }, 100 - elapsed),
    };
    this.pendingLeaderboards.set(lobby.id, holder);
  }

  private send(socket: WebSocket, message: Envelope): void {
    if (socket.readyState === socket.OPEN) socket.send(JSON.stringify(message));
  }
}
