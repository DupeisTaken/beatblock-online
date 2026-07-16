import { Static, Type } from '@sinclair/typebox';
import { Value } from '@sinclair/typebox/value';

export const PROTOCOL_VERSION = 2 as const;
export const MAX_PLAYERS = 16;
export const MAX_SPECTATORS = 32;
export const MAX_RENDER_STREAMS = 4;
export const RENDER_DATAGRAM_BYTES = 32;

export const RoleSchema = Type.Union([
  Type.Literal('player'),
  Type.Literal('spectator'),
  Type.Literal('host'),
]);
export type Role = Static<typeof RoleSchema>;

export const RoomLifecycleSchema = Type.Union([
  Type.Literal('forming'),
  Type.Literal('chart_locked'),
  Type.Literal('ready'),
  Type.Literal('countdown'),
  Type.Literal('playing'),
  Type.Literal('results'),
  Type.Literal('set_complete'),
  Type.Literal('closed'),
]);
export type RoomLifecycle = Static<typeof RoomLifecycleSchema>;

export const RunValiditySchema = Type.Union([
  Type.Literal('pending'),
  Type.Literal('valid'),
  Type.Literal('invalid'),
  Type.Literal('dnf'),
]);
export type RunValidity = Static<typeof RunValiditySchema>;

export const EnvelopeSchema = Type.Object({
  version: Type.Literal(PROTOCOL_VERSION),
  type: Type.String({ minLength: 1, maxLength: 80 }),
  sequence: Type.Integer({ minimum: 0 }),
  runTimeUs: Type.Integer({ minimum: 0 }),
  runId: Type.Optional(Type.String({ maxLength: 80 })),
  requestId: Type.Optional(Type.String({ minLength: 1, maxLength: 80 })),
  payload: Type.Unknown(),
}, { additionalProperties: false });
export type Envelope<T = unknown> = Omit<Static<typeof EnvelopeSchema>, 'payload'> & { payload: T };

export const ScoreTotalsSchema = Type.Object({
  hits: Type.Integer({ minimum: 0 }),
  misses: Type.Integer({ minimum: 0 }),
  barelies: Type.Integer({ minimum: 0 }),
  combo: Type.Integer({ minimum: 0 }),
  maxCombo: Type.Integer({ minimum: 0 }),
  currentMaxHits: Type.Integer({ minimum: 0 }),
  maxHits: Type.Integer({ minimum: 0 }),
  mineHits: Type.Integer({ minimum: 0, default: 0 }),
}, { additionalProperties: false });
export type ScoreTotals = Static<typeof ScoreTotalsSchema>;

export const ChartLockSchema = Type.Object({
  hash: Type.String({ pattern: '^[a-f0-9]{64}$' }),
  packageName: Type.String({ minLength: 1, maxLength: 256 }),
  songName: Type.String({ minLength: 1, maxLength: 256 }),
  variant: Type.String({ maxLength: 128 }),
  expectedMaxHits: Type.Integer({ minimum: 1 }),
  official: Type.Boolean(),
  transferMode: Type.Union([Type.Literal('verify_only'), Type.Literal('host_transfer')]),
});
export type ChartLock = Static<typeof ChartLockSchema>;
export const ChartFingerprintSchema = ChartLockSchema;
export type ChartFingerprint = ChartLock;

export const ParticipantSchema = Type.Object({
  sessionId: Type.String({ minLength: 1 }),
  displayName: Type.String({ minLength: 1, maxLength: 48 }),
  role: RoleSchema,
  admitted: Type.Boolean(),
  connected: Type.Boolean(),
  ready: Type.Boolean(),
  verified: Type.Boolean(),
  rank: Type.Optional(Type.Integer({ minimum: 1 })),
  progress: Type.Number({ minimum: 0, maximum: 1 }),
  accuracy: Type.Number({ minimum: 0, maximum: 100 }),
  setTotal: Type.Number({ minimum: 0 }),
  totals: ScoreTotalsSchema,
  validity: RunValiditySchema,
  invalidReason: Type.Optional(Type.String({ maxLength: 512 })),
});
export type Participant = Static<typeof ParticipantSchema>;
export const PlayerSnapshotSchema = ParticipantSchema;
export type PlayerSnapshot = Participant;

export const RulesSchema = Type.Object({
  rate: Type.Literal(1),
  pauseAllowed: Type.Literal(false),
  retryAllowed: Type.Literal(false),
  forceStart: Type.Boolean(),
}, { additionalProperties: false });
export type Rules = Static<typeof RulesSchema>;

export const RoomSnapshotSchema = Type.Object({
  id: Type.String({ minLength: 1 }),
  name: Type.String({ minLength: 1, maxLength: 80 }),
  hostSessionId: Type.String({ minLength: 1 }),
  lifecycle: RoomLifecycleSchema,
  admissionMode: Type.Union([Type.Literal('password_only'), Type.Literal('host_approval')]),
  chart: Type.Optional(ChartLockSchema),
  participants: Type.Array(ParticipantSchema, { maxItems: MAX_PLAYERS + MAX_SPECTATORS + 1 }),
  scheduledStartTimeMs: Type.Optional(Type.Integer({ minimum: 0 })),
  forceStart: Type.Boolean(),
  setlist: Type.Array(Type.Object({ id: Type.String(), chart: ChartLockSchema, completed: Type.Boolean() })),
  currentSetlistIndex: Type.Union([Type.Integer({ minimum: 0 }), Type.Null()]),
  createdAtMs: Type.Integer({ minimum: 0 }),
  updatedAtMs: Type.Integer({ minimum: 0 }),
});
export type RoomSnapshot = Static<typeof RoomSnapshotSchema>;
export const LobbySnapshotSchema = RoomSnapshotSchema;
export type LobbySnapshot = RoomSnapshot;

export const RunScoreDeltaSchema = Type.Object({
  runSequence: Type.Integer({ minimum: 0 }),
  progress: Type.Number({ minimum: 0, maximum: 1 }),
  beat: Type.Number(),
  songTimeMs: Type.Integer(),
  totals: ScoreTotalsSchema,
});
export type RunScoreDelta = Static<typeof RunScoreDeltaSchema>;

export const ClientHelloSchema = Type.Object({
  clientVersion: Type.String({ minLength: 1 }),
  gameBuildHash: Type.String({ minLength: 1 }),
  distribution: Type.Union([Type.Literal('standalone'), Type.Literal('beatblock-plus')]),
  mods: Type.Array(Type.Object({ id: Type.String(), hash: Type.String() })),
});
export type ClientHello = Static<typeof ClientHelloSchema>;

export interface RenderDatagram {
  sessionId: number;
  sequence: number;
  runTimeUs: bigint;
  beat: number;
  paddleAngle: number;
  tapMask: number;
  flags: number;
}

export const EMPTY_TOTALS: ScoreTotals = {
  hits: 0, misses: 0, barelies: 0, combo: 0, maxCombo: 0,
  currentMaxHits: 0, maxHits: 0, mineHits: 0,
};

export function calculateAccuracy(
  totals: Pick<ScoreTotals, 'misses' | 'barelies' | 'currentMaxHits'>,
): number {
  if (totals.currentMaxHits <= 0) return 100;
  const raw = ((totals.currentMaxHits - totals.misses - totals.barelies / 4) /
    totals.currentMaxHits) * 100;
  return Math.max(0, Math.floor(raw * 100) / 100);
}

export function rankPlayers(players: Participant[]): Participant[] {
  const competitors = players.filter((player) => player.role !== 'spectator').slice().sort((a, b) => {
    const aValid = a.validity === 'valid' || a.validity === 'pending';
    const bValid = b.validity === 'valid' || b.validity === 'pending';
    if (aValid !== bValid) return aValid ? -1 : 1;
    if (b.accuracy !== a.accuracy) return b.accuracy - a.accuracy;
    if (b.progress !== a.progress) return b.progress - a.progress;
    if (b.totals.maxCombo !== a.totals.maxCombo) return b.totals.maxCombo - a.totals.maxCombo;
    return a.displayName.localeCompare(b.displayName);
  });
  const ranks = new Map(competitors.map((player, index) => [player.sessionId, index + 1]));
  return players.map((player) => {
    const rank = ranks.get(player.sessionId);
    return rank === undefined ? player : { ...player, rank };
  });
}

export function envelope<T>(type: string, sequence: number, payload: T, runId?: string, requestId?: string): Envelope<T> {
  return {
    version: PROTOCOL_VERSION, type, sequence,
    runTimeUs: Math.floor(performance.now() * 1000),
    ...(runId === undefined ? {} : { runId }),
    ...(requestId === undefined ? {} : { requestId }),
    payload,
  };
}

export function encodeRenderDatagram(sample: RenderDatagram): Uint8Array {
  const bytes = new Uint8Array(RENDER_DATAGRAM_BYTES);
  const view = new DataView(bytes.buffer);
  view.setUint8(0, PROTOCOL_VERSION);
  view.setUint32(4, sample.sessionId, true);
  view.setUint32(8, sample.sequence, true);
  view.setBigUint64(12, sample.runTimeUs, true);
  view.setFloat32(20, sample.beat, true);
  view.setFloat32(24, sample.paddleAngle, true);
  view.setUint16(28, sample.tapMask, true);
  view.setUint16(30, sample.flags, true);
  return bytes;
}

export function decodeRenderDatagram(bytes: Uint8Array): RenderDatagram {
  if (bytes.byteLength !== RENDER_DATAGRAM_BYTES || bytes[0] !== PROTOCOL_VERSION) {
    throw new Error('protocol.incompatible_render_datagram');
  }
  const view = new DataView(bytes.buffer, bytes.byteOffset, bytes.byteLength);
  return {
    sessionId: view.getUint32(4, true),
    sequence: view.getUint32(8, true),
    runTimeUs: view.getBigUint64(12, true),
    beat: view.getFloat32(20, true),
    paddleAngle: view.getFloat32(24, true),
    tapMask: view.getUint16(28, true),
    flags: view.getUint16(30, true),
  };
}

export const isEnvelope = (value: unknown): value is Envelope => Value.Check(EnvelopeSchema, value);
export const isRunScoreDelta = (value: unknown): value is RunScoreDelta => Value.Check(RunScoreDeltaSchema, value);
export const isChartFingerprint = (value: unknown): value is ChartFingerprint => Value.Check(ChartLockSchema, value);
