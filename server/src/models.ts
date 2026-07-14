import type { Envelope, LobbySnapshot, Role, ScoreTotals } from '@bbt/protocol';

export interface UserRecord {
  id: string;
  displayName: string;
  role: Role;
  disabled: boolean;
  createdAtMs: number;
}

export interface InviteRecord {
  id: string;
  codeHash: string;
  role: Exclude<Role, 'operator'>;
  maxRedemptions: number;
  redemptions: number;
  expiresAtMs?: number;
  revokedAtMs?: number;
  createdAtMs: number;
}

export interface SessionRecord {
  id: string;
  userId: string;
  refreshHash: string;
  deviceName: string;
  expiresAtMs: number;
  revokedAtMs?: number;
  createdAtMs: number;
}

export interface BrowserTicketRecord {
  hash: string;
  userId: string;
  expiresAtMs: number;
  consumedAtMs?: number;
}

export interface RunEventRecord {
  lobbyId: string;
  runId: string;
  userId: string;
  sequence: number;
  receivedAtMs: number;
  envelope: Envelope;
}

export interface RunSummaryRecord {
  runId: string;
  lobbyId: string;
  userId: string;
  accuracy: number;
  progress: number;
  validity: 'pending' | 'valid' | 'invalid' | 'dnf';
  invalidReason?: string;
  totals: ScoreTotals;
  updatedAtMs: number;
}

export interface AuditRecord {
  actorId?: string;
  action: string;
  subjectId?: string;
  metadata?: Record<string, unknown>;
  createdAtMs: number;
}

export interface StoreStatus {
  users: number;
  invites: number;
  lobbies: number;
  runEvents: number;
}

export interface Store {
  migrate(): Promise<void>;
  close(): Promise<void>;
  createUser(user: UserRecord): Promise<void>;
  getUser(id: string): Promise<UserRecord | undefined>;
  getUserByDisplayName(displayName: string): Promise<UserRecord | undefined>;
  listUsers(): Promise<UserRecord[]>;
  updateUser(user: UserRecord): Promise<void>;
  createInvite(invite: InviteRecord): Promise<void>;
  getInviteByHash(hash: string): Promise<InviteRecord | undefined>;
  getInvite(id: string): Promise<InviteRecord | undefined>;
  listInvites(): Promise<InviteRecord[]>;
  updateInvite(invite: InviteRecord): Promise<void>;
  createSession(session: SessionRecord): Promise<void>;
  getSessionByRefreshHash(hash: string): Promise<SessionRecord | undefined>;
  listSessions(userId: string): Promise<SessionRecord[]>;
  updateSession(session: SessionRecord): Promise<void>;
  createBrowserTicket(ticket: BrowserTicketRecord): Promise<void>;
  consumeBrowserTicket(hash: string, nowMs: number): Promise<BrowserTicketRecord | undefined>;
  saveLobby(lobby: LobbySnapshot): Promise<void>;
  getLobby(idOrCode: string): Promise<LobbySnapshot | undefined>;
  listLobbies(): Promise<LobbySnapshot[]>;
  appendRunEvent(event: RunEventRecord): Promise<'inserted' | 'duplicate'>;
  getRunSequenceState(
    runId: string,
  ): Promise<{ min: number; max: number; count: number } | undefined>;
  saveRunSummary(summary: RunSummaryRecord): Promise<void>;
  appendAudit(record: AuditRecord): Promise<void>;
  pruneRunEvents(beforeMs: number): Promise<number>;
  getStatus(): Promise<StoreStatus>;
  addAllowedMod(id: string, hash: string): Promise<void>;
  removeAllowedMod(id: string): Promise<void>;
  listAllowedMods(): Promise<Array<{ id: string; hash: string }>>;
}
