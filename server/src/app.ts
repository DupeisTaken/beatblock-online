import Fastify, { type FastifyRequest } from 'fastify';
import cors from '@fastify/cors';
import rateLimit from '@fastify/rate-limit';
import websocket from '@fastify/websocket';
import { isChartFingerprint, type ClientHello, type Role } from '@bbt/protocol';
import type { Config } from './config.js';
import { AuthError, AuthService } from './auth-service.js';
import { GatewayHub } from './gateway.js';
import { LobbyError, LobbyService } from './lobby-service.js';
import type { Store, UserRecord } from './models.js';
import { verifyAccessToken } from './security.js';

export interface AppOptions {
  config: Config;
  store: Store;
  logger?: boolean;
}

export async function buildApp(options: AppOptions) {
  const app = Fastify({ logger: options.logger ?? true, trustProxy: true });
  const auth = new AuthService(options.store, options.config);
  const lobbies = new LobbyService(options.store, options.config.supportedGameBuilds);
  const hub = new GatewayHub(lobbies);
  await app.register(cors, {
    origin: (origin, callback) =>
      callback(
        null,
        !origin ||
          origin === options.config.publicUrl ||
          /^https?:\/\/(127\.0\.0\.1|localhost)(:\d+)?$/.test(origin),
      ),
    credentials: false,
  });
  await app.register(rateLimit, { max: 120, timeWindow: '1 minute' });
  await app.register(websocket, { options: { maxPayload: 256 * 1024 } });

  const bearer = (request: FastifyRequest): string | undefined =>
    request.headers.authorization?.match(/^Bearer (.+)$/i)?.[1];
  const currentUser = async (request: FastifyRequest): Promise<UserRecord> => {
    const token = bearer(request);
    if (!token) throw new AuthError('Bearer token required');
    const claims = await verifyAccessToken(options.config, token);
    const user = await options.store.getUser(claims.userId);
    if (!user || user.disabled) throw new AuthError('Account is unavailable', 403);
    return user;
  };

  app.setErrorHandler((error, _request, reply) => {
    if (error instanceof AuthError || error instanceof LobbyError)
      return reply.code(error.statusCode).send({ error: error.message });
    app.log.error(error);
    return reply.code(500).send({ error: 'Internal server error' });
  });

  app.get('/health', async () => ({
    ok: true,
    version: '0.1.0-alpha.1',
    protocolVersion: 1,
    nowMs: Date.now(),
  }));
  app.get('/api/v1/instance', async () => ({
    name: 'Beatblock Together',
    publicUrl: options.config.publicUrl,
    maxPlayers: 16,
    maxSpectators: 32,
    protocolVersion: 1,
  }));

  app.post(
    '/api/v1/auth/redeem',
    { config: { rateLimit: { max: 8, timeWindow: '5 minutes' } } },
    async (request, reply) => {
      const body = request.body as {
        inviteCode?: string;
        displayName?: string;
        deviceName?: string;
      };
      if (!body?.inviteCode || !body.displayName)
        return reply.code(400).send({ error: 'inviteCode and displayName are required' });
      return auth.redeem(body.inviteCode, body.displayName, body.deviceName ?? 'Windows PC');
    },
  );
  app.post('/api/v1/auth/refresh', async (request, reply) => {
    const body = request.body as { refreshToken?: string };
    if (!body?.refreshToken) return reply.code(400).send({ error: 'refreshToken is required' });
    return auth.refresh(body.refreshToken);
  });
  app.post('/api/v1/auth/logout', async (request, reply) => {
    const token = bearer(request);
    if (!token) return reply.code(204).send();
    const claims = await verifyAccessToken(options.config, token);
    await auth.revokeSession(claims.sessionId, claims.userId);
    return reply.code(204).send();
  });
  app.post('/api/v1/auth/browser-ticket', async (request) => ({
    ticket: await auth.createBrowserTicket((await currentUser(request)).id),
    expiresInSeconds: 60,
  }));
  app.post('/api/v1/auth/browser-exchange', async (request, reply) => {
    const body = request.body as { ticket?: string };
    if (!body?.ticket) return reply.code(400).send({ error: 'ticket is required' });
    return auth.exchangeBrowserTicket(body.ticket);
  });

  app.post('/api/v1/lobbies', async (request) => {
    const user = await currentUser(request);
    const body = request.body as { name?: string };
    const lobby = await lobbies.create(user, body?.name ?? 'Competition lobby');
    hub.broadcastLobby(lobby);
    return lobby;
  });
  app.get('/api/v1/lobbies/:id', async (request, reply) => {
    await currentUser(request);
    const lobby = await options.store.getLobby((request.params as { id: string }).id);
    return lobby ?? reply.code(404).send({ error: 'Lobby not found' });
  });
  app.post('/api/v1/lobbies/:id/join', async (request) => {
    const user = await currentUser(request);
    const body = request.body as { spectator?: boolean };
    const lobby = await lobbies.join(
      (request.params as { id: string }).id,
      user,
      body?.spectator ?? false,
    );
    hub.broadcastLobby(lobby);
    return lobby;
  });
  app.put('/api/v1/lobbies/:id/chart', async (request) => {
    const user = await currentUser(request);
    if (!isChartFingerprint(request.body))
      throw new LobbyError('Chart fingerprint does not conform to protocol v1');
    const lobby = await lobbies.setChart((request.params as { id: string }).id, user, request.body);
    hub.broadcastLobby(lobby);
    return lobby;
  });
  app.put('/api/v1/lobbies/:id/ready', async (request) => {
    const user = await currentUser(request);
    const body = request.body as { ready: boolean; fingerprint?: string; client?: ClientHello };
    const lobby = await lobbies.setReady(
      (request.params as { id: string }).id,
      user.id,
      body.ready,
      body.fingerprint,
      body.client,
    );
    hub.broadcastLobby(lobby);
    return lobby;
  });
  app.post('/api/v1/lobbies/:id/start', async (request) => {
    const user = await currentUser(request);
    const lobby = await lobbies.scheduleStart((request.params as { id: string }).id, user);
    hub.broadcastLobby(lobby);
    return lobby;
  });
  app.post('/api/v1/lobbies/:id/leave', async (request) => {
    const user = await currentUser(request);
    const lobby = await lobbies.leave((request.params as { id: string }).id, user);
    hub.broadcastLobby(lobby);
    return lobby;
  });
  app.post('/api/v1/lobbies/:id/close', async (request) => {
    const user = await currentUser(request);
    const lobby = await lobbies.close((request.params as { id: string }).id, user);
    hub.broadcastLobby(lobby);
    return lobby;
  });

  app.get('/api/v1/gateway', { websocket: true }, async (socket, request) => {
    try {
      const url = new URL(request.url, options.config.publicUrl);
      const token = url.searchParams.get('access_token');
      if (!token) throw new AuthError('access_token is required');
      const claims = await verifyAccessToken(options.config, token);
      const user = await options.store.getUser(claims.userId);
      if (!user || user.disabled) throw new AuthError('Account unavailable', 403);
      const connection = hub.add(socket, user);
      socket.send(
        JSON.stringify({
          version: 1,
          type: 'gateway.ready',
          sequence: 0,
          timestampMs: Date.now(),
          payload: { userId: user.id, serverTimeMs: Date.now() },
        }),
      );
      socket.on('message', (data) => void hub.receive(connection, data.toString()));
    } catch (error) {
      socket.close(1008, error instanceof Error ? error.message : 'Unauthorized');
    }
  });

  app.get('/api/v1/operator/status', async (request) => {
    const user = await currentUser(request);
    if (user.role !== 'operator') throw new AuthError('Operator permission required', 403);
    return options.store.getStatus();
  });

  app.addHook('onClose', async () => options.store.close());
  return app;
}
