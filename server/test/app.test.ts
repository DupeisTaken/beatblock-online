import { afterEach, beforeEach, describe, expect, it } from 'vitest';
import type { FastifyInstance } from 'fastify';
import { buildApp } from '../src/app.js';
import { AuthService } from '../src/auth-service.js';
import type { Config } from '../src/config.js';
import { MemoryStore } from '../src/memory-store.js';

const config: Config = {
  host: '127.0.0.1',
  port: 0,
  publicUrl: 'http://test',
  databaseUrl: 'memory://',
  tokenSecret: 'test-token-secret-that-is-long-enough',
  inviteSecret: 'test-invite-secret-that-is-long-enough',
  accessTokenTtlSeconds: 900,
  refreshTokenTtlDays: 30,
  runEventRetentionDays: 30,
  allowInsecureHttp: true,
  supportedGameBuilds: ['supported-build'],
};

describe('alpha API', () => {
  let app: FastifyInstance;
  let store: MemoryStore;
  let auth: AuthService;
  beforeEach(async () => {
    store = new MemoryStore();
    auth = new AuthService(store, config);
    app = await buildApp({ config, store, logger: false });
  });
  afterEach(async () => app.close());

  it('redeems a one-use invite once', async () => {
    const created = await auth.createInvite('player', 1, Date.now() + 60_000);
    const first = await app.inject({
      method: 'POST',
      url: '/api/v1/auth/redeem',
      payload: { inviteCode: created.code, displayName: 'Cranky', deviceName: 'Test' },
    });
    expect(first.statusCode).toBe(200);
    const second = await app.inject({
      method: 'POST',
      url: '/api/v1/auth/redeem',
      payload: { inviteCode: created.code, displayName: 'Cranky Two', deviceName: 'Test' },
    });
    expect(second.statusCode).toBe(401);
  });

  it('allows organizers, but not players, to create lobbies', async () => {
    const organizerInvite = await auth.createInvite('organizer', 1);
    const playerInvite = await auth.createInvite('player', 1);
    const organizer = (
      await app.inject({
        method: 'POST',
        url: '/api/v1/auth/redeem',
        payload: { inviteCode: organizerInvite.code, displayName: 'Host' },
      })
    ).json();
    const player = (
      await app.inject({
        method: 'POST',
        url: '/api/v1/auth/redeem',
        payload: { inviteCode: playerInvite.code, displayName: 'Player' },
      })
    ).json();
    expect(
      (
        await app.inject({
          method: 'POST',
          url: '/api/v1/lobbies',
          headers: { authorization: `Bearer ${organizer.accessToken}` },
          payload: { name: 'Finals' },
        })
      ).statusCode,
    ).toBe(200);
    expect(
      (
        await app.inject({
          method: 'POST',
          url: '/api/v1/lobbies',
          headers: { authorization: `Bearer ${player.accessToken}` },
          payload: { name: 'Nope' },
        })
      ).statusCode,
    ).toBe(403);
  });
});
