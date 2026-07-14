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
import type { LobbySnapshot } from '@bbt/protocol';

export class MemoryStore implements Store {
  private users = new Map<string, UserRecord>();
  private invites = new Map<string, InviteRecord>();
  private sessions = new Map<string, SessionRecord>();
  private tickets = new Map<string, BrowserTicketRecord>();
  private lobbies = new Map<string, LobbySnapshot>();
  private events = new Map<string, RunEventRecord>();
  private runs = new Map<string, RunSummaryRecord>();
  private audits: AuditRecord[] = [];
  private allowedMods = new Map<string, string>();

  async migrate(): Promise<void> {}
  async close(): Promise<void> {}
  async createUser(user: UserRecord): Promise<void> {
    this.users.set(user.id, structuredClone(user));
  }
  async getUser(id: string): Promise<UserRecord | undefined> {
    return structuredClone(this.users.get(id));
  }
  async getUserByDisplayName(displayName: string): Promise<UserRecord | undefined> {
    const normalized = displayName.toLocaleLowerCase();
    return structuredClone(
      [...this.users.values()].find((user) => user.displayName.toLocaleLowerCase() === normalized),
    );
  }
  async listUsers(): Promise<UserRecord[]> {
    return structuredClone([...this.users.values()]);
  }
  async updateUser(user: UserRecord): Promise<void> {
    this.users.set(user.id, structuredClone(user));
  }
  async createInvite(invite: InviteRecord): Promise<void> {
    this.invites.set(invite.id, structuredClone(invite));
  }
  async getInviteByHash(hash: string): Promise<InviteRecord | undefined> {
    return structuredClone([...this.invites.values()].find((invite) => invite.codeHash === hash));
  }
  async getInvite(id: string): Promise<InviteRecord | undefined> {
    return structuredClone(this.invites.get(id));
  }
  async listInvites(): Promise<InviteRecord[]> {
    return structuredClone([...this.invites.values()]);
  }
  async updateInvite(invite: InviteRecord): Promise<void> {
    this.invites.set(invite.id, structuredClone(invite));
  }
  async createSession(session: SessionRecord): Promise<void> {
    this.sessions.set(session.id, structuredClone(session));
  }
  async getSessionByRefreshHash(hash: string): Promise<SessionRecord | undefined> {
    return structuredClone(
      [...this.sessions.values()].find((session) => session.refreshHash === hash),
    );
  }
  async listSessions(userId: string): Promise<SessionRecord[]> {
    return structuredClone(
      [...this.sessions.values()].filter((session) => session.userId === userId),
    );
  }
  async updateSession(session: SessionRecord): Promise<void> {
    this.sessions.set(session.id, structuredClone(session));
  }
  async createBrowserTicket(ticket: BrowserTicketRecord): Promise<void> {
    this.tickets.set(ticket.hash, structuredClone(ticket));
  }
  async consumeBrowserTicket(
    hash: string,
    nowMs: number,
  ): Promise<BrowserTicketRecord | undefined> {
    const ticket = this.tickets.get(hash);
    if (!ticket || ticket.consumedAtMs || ticket.expiresAtMs <= nowMs) return undefined;
    ticket.consumedAtMs = nowMs;
    return structuredClone(ticket);
  }
  async saveLobby(lobby: LobbySnapshot): Promise<void> {
    this.lobbies.set(lobby.id, structuredClone(lobby));
  }
  async getLobby(idOrCode: string): Promise<LobbySnapshot | undefined> {
    const lobby =
      this.lobbies.get(idOrCode) ??
      [...this.lobbies.values()].find((item) => item.code === idOrCode.toUpperCase());
    return structuredClone(lobby);
  }
  async listLobbies(): Promise<LobbySnapshot[]> {
    return structuredClone([...this.lobbies.values()]);
  }
  async appendRunEvent(event: RunEventRecord): Promise<'inserted' | 'duplicate'> {
    const key = `${event.runId}:${event.sequence}`;
    if (this.events.has(key)) return 'duplicate';
    this.events.set(key, structuredClone(event));
    return 'inserted';
  }
  async getRunSequenceState(
    runId: string,
  ): Promise<{ min: number; max: number; count: number } | undefined> {
    const values = [...this.events.values()]
      .filter((event) => event.runId === runId)
      .map((event) => event.sequence);
    return values.length
      ? { min: Math.min(...values), max: Math.max(...values), count: values.length }
      : undefined;
  }
  async saveRunSummary(summary: RunSummaryRecord): Promise<void> {
    this.runs.set(summary.runId, structuredClone(summary));
  }
  async appendAudit(record: AuditRecord): Promise<void> {
    this.audits.push(structuredClone(record));
  }
  async pruneRunEvents(beforeMs: number): Promise<number> {
    let count = 0;
    for (const [key, event] of this.events) {
      if (event.receivedAtMs < beforeMs) {
        this.events.delete(key);
        count += 1;
      }
    }
    return count;
  }
  async getStatus(): Promise<StoreStatus> {
    return {
      users: this.users.size,
      invites: this.invites.size,
      lobbies: this.lobbies.size,
      runEvents: this.events.size,
    };
  }
  async addAllowedMod(id: string, hash: string): Promise<void> {
    this.allowedMods.set(id, hash);
  }
  async removeAllowedMod(id: string): Promise<void> {
    this.allowedMods.delete(id);
  }
  async listAllowedMods(): Promise<Array<{ id: string; hash: string }>> {
    return [...this.allowedMods].map(([id, hash]) => ({ id, hash }));
  }
}
