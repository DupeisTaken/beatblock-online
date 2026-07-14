import postgres, { type Sql } from 'postgres';
import type { LobbySnapshot } from '@bbt/protocol';
import type {
  AuditRecord,
  BrowserTicketRecord,
  InviteRecord,
  RunEventRecord,
  RunSummaryRecord,
  SessionRecord,
  Store,
  StoreStatus,
  UserRecord,
} from './models.js';

const asNumber = (value: unknown): number => Number(value);

export class PostgresStore implements Store {
  private readonly sql: Sql;
  constructor(databaseUrl: string) {
    this.sql = postgres(databaseUrl, { max: 10 });
  }

  async migrate(): Promise<void> {
    await this.sql.unsafe(`
      create table if not exists users (
        id text primary key, display_name text not null unique, role text not null,
        disabled boolean not null default false, created_at_ms bigint not null
      );
      create table if not exists invites (
        id text primary key, code_hash text not null unique, role text not null,
        max_redemptions integer not null, redemptions integer not null default 0,
        expires_at_ms bigint, revoked_at_ms bigint, created_at_ms bigint not null
      );
      create table if not exists sessions (
        id text primary key, user_id text not null references users(id), refresh_hash text not null unique,
        device_name text not null, expires_at_ms bigint not null, revoked_at_ms bigint, created_at_ms bigint not null
      );
      create table if not exists browser_tickets (
        hash text primary key, user_id text not null references users(id), expires_at_ms bigint not null, consumed_at_ms bigint
      );
      create table if not exists lobbies (
        id text primary key, code text not null unique, lifecycle text not null, snapshot jsonb not null, updated_at_ms bigint not null
      );
      create table if not exists run_events (
        run_id text not null, sequence bigint not null, lobby_id text not null, user_id text not null,
        received_at_ms bigint not null, envelope jsonb not null, primary key (run_id, sequence)
      );
      create index if not exists run_events_received_idx on run_events(received_at_ms);
      create table if not exists runs (
        run_id text primary key, lobby_id text not null, user_id text not null,
        accuracy double precision not null, progress double precision not null,
        validity text not null, invalid_reason text, totals jsonb not null, updated_at_ms bigint not null
      );
      create table if not exists audit_log (
        id bigserial primary key, actor_id text, action text not null, subject_id text,
        metadata jsonb, created_at_ms bigint not null
      );
      create table if not exists allowed_mods (id text primary key, hash text not null);
    `);
  }

  async close(): Promise<void> {
    await this.sql.end();
  }
  async createUser(user: UserRecord): Promise<void> {
    await this
      .sql`insert into users ${this.sql({ id: user.id, display_name: user.displayName, role: user.role, disabled: user.disabled, created_at_ms: user.createdAtMs })}`;
  }
  async getUser(id: string): Promise<UserRecord | undefined> {
    return this.mapUser((await this.sql`select * from users where id=${id}`)[0]);
  }
  async getUserByDisplayName(name: string): Promise<UserRecord | undefined> {
    return this.mapUser(
      (await this.sql`select * from users where lower(display_name)=lower(${name})`)[0],
    );
  }
  async listUsers(): Promise<UserRecord[]> {
    return (await this.sql`select * from users order by created_at_ms`).map((row) =>
      this.mapUser(row)!,
    );
  }
  async updateUser(user: UserRecord): Promise<void> {
    await this
      .sql`update users set display_name=${user.displayName}, role=${user.role}, disabled=${user.disabled} where id=${user.id}`;
  }
  private mapUser(row: Record<string, unknown> | undefined): UserRecord | undefined {
    if (!row) return undefined;
    return {
      id: String(row.id),
      displayName: String(row.display_name),
      role: row.role as UserRecord['role'],
      disabled: Boolean(row.disabled),
      createdAtMs: asNumber(row.created_at_ms),
    };
  }
  async createInvite(invite: InviteRecord): Promise<void> {
    await this
      .sql`insert into invites ${this.sql({ id: invite.id, code_hash: invite.codeHash, role: invite.role, max_redemptions: invite.maxRedemptions, redemptions: invite.redemptions, expires_at_ms: invite.expiresAtMs ?? null, revoked_at_ms: invite.revokedAtMs ?? null, created_at_ms: invite.createdAtMs })}`;
  }
  async getInviteByHash(hash: string): Promise<InviteRecord | undefined> {
    return this.mapInvite((await this.sql`select * from invites where code_hash=${hash}`)[0]);
  }
  async getInvite(id: string): Promise<InviteRecord | undefined> {
    return this.mapInvite((await this.sql`select * from invites where id=${id}`)[0]);
  }
  async listInvites(): Promise<InviteRecord[]> {
    return (await this.sql`select * from invites order by created_at_ms desc`).map((row) =>
      this.mapInvite(row)!,
    );
  }
  async updateInvite(invite: InviteRecord): Promise<void> {
    await this
      .sql`update invites set redemptions=${invite.redemptions}, revoked_at_ms=${invite.revokedAtMs ?? null} where id=${invite.id}`;
  }
  private mapInvite(row: Record<string, unknown> | undefined): InviteRecord | undefined {
    if (!row) return undefined;
    const value: InviteRecord = {
      id: String(row.id),
      codeHash: String(row.code_hash),
      role: row.role as InviteRecord['role'],
      maxRedemptions: asNumber(row.max_redemptions),
      redemptions: asNumber(row.redemptions),
      createdAtMs: asNumber(row.created_at_ms),
    };
    if (row.expires_at_ms != null) value.expiresAtMs = asNumber(row.expires_at_ms);
    if (row.revoked_at_ms != null) value.revokedAtMs = asNumber(row.revoked_at_ms);
    return value;
  }
  async createSession(session: SessionRecord): Promise<void> {
    await this
      .sql`insert into sessions ${this.sql({ id: session.id, user_id: session.userId, refresh_hash: session.refreshHash, device_name: session.deviceName, expires_at_ms: session.expiresAtMs, revoked_at_ms: session.revokedAtMs ?? null, created_at_ms: session.createdAtMs })}`;
  }
  async getSessionByRefreshHash(hash: string): Promise<SessionRecord | undefined> {
    return this.mapSession((await this.sql`select * from sessions where refresh_hash=${hash}`)[0]);
  }
  async listSessions(userId: string): Promise<SessionRecord[]> {
    return (await this.sql`select * from sessions where user_id=${userId}`).map((row) =>
      this.mapSession(row)!,
    );
  }
  async updateSession(session: SessionRecord): Promise<void> {
    await this
      .sql`update sessions set refresh_hash=${session.refreshHash}, expires_at_ms=${session.expiresAtMs}, revoked_at_ms=${session.revokedAtMs ?? null} where id=${session.id}`;
  }
  private mapSession(row: Record<string, unknown> | undefined): SessionRecord | undefined {
    if (!row) return undefined;
    const value: SessionRecord = {
      id: String(row.id),
      userId: String(row.user_id),
      refreshHash: String(row.refresh_hash),
      deviceName: String(row.device_name),
      expiresAtMs: asNumber(row.expires_at_ms),
      createdAtMs: asNumber(row.created_at_ms),
    };
    if (row.revoked_at_ms != null) value.revokedAtMs = asNumber(row.revoked_at_ms);
    return value;
  }
  async createBrowserTicket(ticket: BrowserTicketRecord): Promise<void> {
    await this
      .sql`insert into browser_tickets ${this.sql({ hash: ticket.hash, user_id: ticket.userId, expires_at_ms: ticket.expiresAtMs, consumed_at_ms: ticket.consumedAtMs ?? null })}`;
  }
  async consumeBrowserTicket(
    hash: string,
    nowMs: number,
  ): Promise<BrowserTicketRecord | undefined> {
    const rows = await this
      .sql`update browser_tickets set consumed_at_ms=${nowMs} where hash=${hash} and consumed_at_ms is null and expires_at_ms>${nowMs} returning *`;
    const row = rows[0];
    return row
      ? {
          hash: String(row.hash),
          userId: String(row.user_id),
          expiresAtMs: asNumber(row.expires_at_ms),
          consumedAtMs: asNumber(row.consumed_at_ms),
        }
      : undefined;
  }
  async saveLobby(lobby: LobbySnapshot): Promise<void> {
    await this
      .sql`insert into lobbies (id, code, lifecycle, snapshot, updated_at_ms) values (${lobby.id}, ${lobby.code}, ${lobby.lifecycle}, ${this.sql.json(lobby as never)}, ${lobby.updatedAtMs}) on conflict(id) do update set code=excluded.code, lifecycle=excluded.lifecycle, snapshot=excluded.snapshot, updated_at_ms=excluded.updated_at_ms`;
  }
  async getLobby(idOrCode: string): Promise<LobbySnapshot | undefined> {
    const row = (
      await this
        .sql`select snapshot from lobbies where id=${idOrCode} or code=${idOrCode.toUpperCase()} limit 1`
    )[0];
    return row?.snapshot as LobbySnapshot | undefined;
  }
  async listLobbies(): Promise<LobbySnapshot[]> {
    return (await this.sql`select snapshot from lobbies order by updated_at_ms desc`).map(
      (row) => row.snapshot as LobbySnapshot,
    );
  }
  async appendRunEvent(event: RunEventRecord): Promise<'inserted' | 'duplicate'> {
    const rows = await this
      .sql`insert into run_events (run_id, sequence, lobby_id, user_id, received_at_ms, envelope) values (${event.runId}, ${event.sequence}, ${event.lobbyId}, ${event.userId}, ${event.receivedAtMs}, ${this.sql.json(event.envelope as never)}) on conflict do nothing returning run_id`;
    return rows.length ? 'inserted' : 'duplicate';
  }
  async getRunSequenceState(
    runId: string,
  ): Promise<{ min: number; max: number; count: number } | undefined> {
    const row = (
      await this
        .sql`select min(sequence) as min, max(sequence) as max, count(*) as count from run_events where run_id=${runId}`
    )[0];
    return row && asNumber(row.count) > 0
      ? { min: asNumber(row.min), max: asNumber(row.max), count: asNumber(row.count) }
      : undefined;
  }
  async saveRunSummary(summary: RunSummaryRecord): Promise<void> {
    await this
      .sql`insert into runs (run_id, lobby_id, user_id, accuracy, progress, validity, invalid_reason, totals, updated_at_ms) values (${summary.runId}, ${summary.lobbyId}, ${summary.userId}, ${summary.accuracy}, ${summary.progress}, ${summary.validity}, ${summary.invalidReason ?? null}, ${this.sql.json(summary.totals as never)}, ${summary.updatedAtMs}) on conflict(run_id) do update set accuracy=excluded.accuracy, progress=excluded.progress, validity=excluded.validity, invalid_reason=excluded.invalid_reason, totals=excluded.totals, updated_at_ms=excluded.updated_at_ms`;
  }
  async appendAudit(record: AuditRecord): Promise<void> {
    await this
      .sql`insert into audit_log (actor_id, action, subject_id, metadata, created_at_ms) values (${record.actorId ?? null}, ${record.action}, ${record.subjectId ?? null}, ${record.metadata ? this.sql.json(record.metadata as never) : null}, ${record.createdAtMs})`;
  }
  async pruneRunEvents(beforeMs: number): Promise<number> {
    return (await this.sql`delete from run_events where received_at_ms<${beforeMs}`).count;
  }
  async getStatus(): Promise<StoreStatus> {
    const [users, invites, lobbies, runEvents] = await Promise.all([
      this.sql`select count(*) from users`,
      this.sql`select count(*) from invites`,
      this.sql`select count(*) from lobbies`,
      this.sql`select count(*) from run_events`,
    ]);
    return {
      users: asNumber(users[0]?.count),
      invites: asNumber(invites[0]?.count),
      lobbies: asNumber(lobbies[0]?.count),
      runEvents: asNumber(runEvents[0]?.count),
    };
  }
  async addAllowedMod(id: string, hash: string): Promise<void> {
    await this
      .sql`insert into allowed_mods (id, hash) values (${id},${hash}) on conflict(id) do update set hash=excluded.hash`;
  }
  async removeAllowedMod(id: string): Promise<void> {
    await this.sql`delete from allowed_mods where id=${id}`;
  }
  async listAllowedMods(): Promise<Array<{ id: string; hash: string }>> {
    return (await this.sql`select id, hash from allowed_mods order by id`).map((row) => ({
      id: String(row.id),
      hash: String(row.hash),
    }));
  }
}
