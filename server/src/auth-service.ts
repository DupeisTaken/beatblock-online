import type { Role } from '@bbt/protocol';
import type { Config } from './config.js';
import type { InviteRecord, Store, UserRecord } from './models.js';
import { hashSecret, id, opaqueToken, signAccessToken } from './security.js';

export class AuthError extends Error {
  constructor(
    message: string,
    public statusCode = 401,
  ) {
    super(message);
  }
}

export class AuthService {
  constructor(
    private store: Store,
    private config: Config,
  ) {}

  async redeem(inviteCode: string, displayName: string, deviceName: string) {
    const normalizedName = displayName.trim();
    if (!/^[\p{L}\p{N} _.-]{3,32}$/u.test(normalizedName))
      throw new AuthError('Display name must be 3–32 safe characters', 400);
    if (await this.store.getUserByDisplayName(normalizedName))
      throw new AuthError('Display name is already in use', 409);
    const codeHash = hashSecret(this.config.inviteSecret, inviteCode.trim().toUpperCase());
    const invite = await this.store.getInviteByHash(codeHash);
    const now = Date.now();
    if (
      !invite ||
      invite.revokedAtMs ||
      (invite.expiresAtMs && invite.expiresAtMs <= now) ||
      invite.redemptions >= invite.maxRedemptions
    ) {
      throw new AuthError('Invite is invalid, expired, revoked, or exhausted');
    }
    const user: UserRecord = {
      id: id(),
      displayName: normalizedName,
      role: invite.role,
      disabled: false,
      createdAtMs: now,
    };
    const refreshToken = opaqueToken();
    const sessionId = id();
    await this.store.createUser(user);
    await this.store.createSession({
      id: sessionId,
      userId: user.id,
      refreshHash: hashSecret(this.config.tokenSecret, refreshToken),
      deviceName: deviceName.trim().slice(0, 80) || 'Windows PC',
      expiresAtMs: now + this.config.refreshTokenTtlDays * 86_400_000,
      createdAtMs: now,
    });
    invite.redemptions += 1;
    await this.store.updateInvite(invite);
    await this.store.appendAudit({
      actorId: user.id,
      action: 'invite.redeemed',
      subjectId: invite.id,
      createdAtMs: now,
    });
    return {
      accessToken: await signAccessToken(this.config, {
        userId: user.id,
        role: user.role,
        sessionId,
      }),
      refreshToken,
      expiresInSeconds: this.config.accessTokenTtlSeconds,
      user,
    };
  }

  async refresh(refreshToken: string) {
    const now = Date.now();
    const session = await this.store.getSessionByRefreshHash(
      hashSecret(this.config.tokenSecret, refreshToken),
    );
    if (!session || session.revokedAtMs || session.expiresAtMs <= now)
      throw new AuthError('Refresh credential is invalid');
    const user = await this.store.getUser(session.userId);
    if (!user || user.disabled) throw new AuthError('Account is disabled', 403);
    const nextRefresh = opaqueToken();
    session.refreshHash = hashSecret(this.config.tokenSecret, nextRefresh);
    session.expiresAtMs = now + this.config.refreshTokenTtlDays * 86_400_000;
    await this.store.updateSession(session);
    return {
      accessToken: await signAccessToken(this.config, {
        userId: user.id,
        role: user.role,
        sessionId: session.id,
      }),
      refreshToken: nextRefresh,
      expiresInSeconds: this.config.accessTokenTtlSeconds,
      user,
    };
  }

  async revokeSession(sessionId: string, actorId?: string): Promise<void> {
    const users = await this.store.listUsers();
    for (const user of users) {
      const session = (await this.store.listSessions(user.id)).find(
        (item) => item.id === sessionId,
      );
      if (session) {
        session.revokedAtMs = Date.now();
        await this.store.updateSession(session);
        await this.store.appendAudit({
          ...(actorId ? { actorId } : {}),
          action: 'session.revoked',
          subjectId: sessionId,
          createdAtMs: Date.now(),
        });
        return;
      }
    }
  }

  async createBrowserTicket(userId: string): Promise<string> {
    const ticket = opaqueToken(24);
    await this.store.createBrowserTicket({
      hash: hashSecret(this.config.tokenSecret, ticket),
      userId,
      expiresAtMs: Date.now() + 60_000,
    });
    return ticket;
  }

  async exchangeBrowserTicket(ticket: string) {
    const record = await this.store.consumeBrowserTicket(
      hashSecret(this.config.tokenSecret, ticket),
      Date.now(),
    );
    if (!record) throw new AuthError('Browser ticket is invalid or expired');
    const user = await this.store.getUser(record.userId);
    if (!user || user.disabled) throw new AuthError('Account is disabled', 403);
    const sessionId = `browser:${id()}`;
    return {
      accessToken: await signAccessToken(this.config, {
        userId: user.id,
        role: user.role,
        sessionId,
      }),
      expiresInSeconds: this.config.accessTokenTtlSeconds,
      user,
    };
  }

  async createInvite(
    role: Exclude<Role, 'operator'>,
    maxRedemptions: number,
    expiresAtMs?: number,
  ): Promise<{ code: string; invite: InviteRecord }> {
    const code = (await import('./security.js')).createInviteCode();
    const invite: InviteRecord = {
      id: id(),
      codeHash: hashSecret(this.config.inviteSecret, code),
      role,
      maxRedemptions,
      redemptions: 0,
      createdAtMs: Date.now(),
    };
    if (expiresAtMs) invite.expiresAtMs = expiresAtMs;
    await this.store.createInvite(invite);
    return { code, invite };
  }
}
