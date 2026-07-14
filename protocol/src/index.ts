import { Static, Type } from '@sinclair/typebox';
import { Value } from '@sinclair/typebox/value';

export const PROTOCOL_VERSION = 1 as const;
export const MAX_PLAYERS = 16;
export const MAX_SPECTATORS = 32;

export const RoleSchema = Type.Union([
  Type.Literal('player'),
  Type.Literal('organizer'),
  Type.Literal('operator'),
]);
export type Role = Static<typeof RoleSchema>;

export const LobbyLifecycleSchema = Type.Union([
  Type.Literal('forming'),
  Type.Literal('chart_locked'),
  Type.Literal('ready'),
  Type.Literal('countdown'),
  Type.Literal('playing'),
  Type.Literal('results'),
  Type.Literal('closed'),
]);
export type LobbyLifecycle = Static<typeof LobbyLifecycleSchema>;

export const RunValiditySchema = Type.Union([
  Type.Literal('pending'),
  Type.Literal('valid'),
  Type.Literal('invalid'),
  Type.Literal('dnf'),
]);
export type RunValidity = Static<typeof RunValiditySchema>;

export const EnvelopeSchema = Type.Object(
  {
    version: Type.Literal(PROTOCOL_VERSION),
    type: Type.String({ minLength: 1, maxLength: 80 }),
    sequence: Type.Integer({ minimum: 0 }),
    timestampMs: Type.Integer({ minimum: 0 }),
    payload: Type.Unknown(),
  },
  { additionalProperties: false },
);
export type Envelope<T = unknown> = Omit<Static<typeof EnvelopeSchema>, 'payload'> & { payload: T };

export const ScoreTotalsSchema = Type.Object(
  {
    hits: Type.Integer({ minimum: 0 }),
    misses: Type.Integer({ minimum: 0 }),
    barelies: Type.Integer({ minimum: 0 }),
    combo: Type.Integer({ minimum: 0 }),
    maxCombo: Type.Integer({ minimum: 0 }),
    currentMaxHits: Type.Integer({ minimum: 0 }),
    maxHits: Type.Integer({ minimum: 0 }),
    mineHits: Type.Integer({ minimum: 0, default: 0 }),
  },
  { additionalProperties: false },
);
export type ScoreTotals = Static<typeof ScoreTotalsSchema>;

export const ChartFingerprintSchema = Type.Object(
  {
    algorithm: Type.Literal('sha256-canonical-package-v1'),
    hash: Type.String({ pattern: '^[a-f0-9]{64}$' }),
    packageName: Type.String({ minLength: 1, maxLength: 256 }),
    songName: Type.String({ minLength: 1, maxLength: 256 }),
    variant: Type.String({ maxLength: 128 }),
    expectedMaxHits: Type.Integer({ minimum: 1 }),
  },
  { additionalProperties: false },
);
export type ChartFingerprint = Static<typeof ChartFingerprintSchema>;

export const RulesSchema = Type.Object(
  {
    rate: Type.Literal(1),
    taps: Type.Literal('default'),
    sides: Type.Literal('default'),
    barelies: Type.Literal('default'),
    pauseAllowed: Type.Literal(false),
    retryAllowed: Type.Literal(false),
  },
  { additionalProperties: false },
);
export type Rules = Static<typeof RulesSchema>;

export const PlayerSnapshotSchema = Type.Object({
  userId: Type.String({ minLength: 1 }),
  displayName: Type.String({ minLength: 1, maxLength: 48 }),
  connected: Type.Boolean(),
  ready: Type.Boolean(),
  spectator: Type.Boolean(),
  rank: Type.Optional(Type.Integer({ minimum: 1 })),
  progress: Type.Number({ minimum: 0, maximum: 1 }),
  accuracy: Type.Number({ minimum: 0, maximum: 100 }),
  totals: ScoreTotalsSchema,
  validity: RunValiditySchema,
  invalidReason: Type.Optional(Type.String({ maxLength: 256 })),
});
export type PlayerSnapshot = Static<typeof PlayerSnapshotSchema>;

export const LobbySnapshotSchema = Type.Object({
  id: Type.String({ minLength: 1 }),
  code: Type.String({ minLength: 6, maxLength: 8 }),
  name: Type.String({ minLength: 1, maxLength: 80 }),
  organizerId: Type.String({ minLength: 1 }),
  lifecycle: LobbyLifecycleSchema,
  chart: Type.Optional(ChartFingerprintSchema),
  rules: RulesSchema,
  players: Type.Array(PlayerSnapshotSchema, { maxItems: MAX_PLAYERS + MAX_SPECTATORS }),
  scheduledStartTimeMs: Type.Optional(Type.Integer({ minimum: 0 })),
  createdAtMs: Type.Integer({ minimum: 0 }),
  updatedAtMs: Type.Integer({ minimum: 0 }),
});
export type LobbySnapshot = Static<typeof LobbySnapshotSchema>;

export const RunScoreDeltaSchema = Type.Object({
  lobbyId: Type.String({ minLength: 1 }),
  runId: Type.String({ minLength: 1 }),
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

export const DEFAULT_RULES: Rules = {
  rate: 1,
  taps: 'default',
  sides: 'default',
  barelies: 'default',
  pauseAllowed: false,
  retryAllowed: false,
};

export const EMPTY_TOTALS: ScoreTotals = {
  hits: 0,
  misses: 0,
  barelies: 0,
  combo: 0,
  maxCombo: 0,
  currentMaxHits: 0,
  maxHits: 0,
  mineHits: 0,
};

export function calculateAccuracy(
  totals: Pick<ScoreTotals, 'misses' | 'barelies' | 'currentMaxHits'>,
): number {
  if (totals.currentMaxHits <= 0) return 100;
  const raw =
    ((totals.currentMaxHits - totals.misses - totals.barelies / 4) / totals.currentMaxHits) * 100;
  return Math.max(0, Math.floor(raw * 100) / 100);
}

export function rankPlayers(players: PlayerSnapshot[]): PlayerSnapshot[] {
  const competitors = players
    .filter((player) => !player.spectator)
    .slice()
    .sort((left, right) => {
      if (left.validity === 'invalid' && right.validity !== 'invalid') return 1;
      if (right.validity === 'invalid' && left.validity !== 'invalid') return -1;
      if (right.accuracy !== left.accuracy) return right.accuracy - left.accuracy;
      if (right.progress !== left.progress) return right.progress - left.progress;
      if (right.totals.maxCombo !== left.totals.maxCombo) {
        return right.totals.maxCombo - left.totals.maxCombo;
      }
      return left.displayName.localeCompare(right.displayName);
    });
  const ranks = new Map(competitors.map((player, index) => [player.userId, index + 1]));
  return players.map((player) => {
    const rank = ranks.get(player.userId);
    return rank === undefined ? player : { ...player, rank };
  });
}

export function envelope<T>(type: string, sequence: number, payload: T): Envelope<T> {
  return { version: PROTOCOL_VERSION, type, sequence, timestampMs: Date.now(), payload };
}

export function isEnvelope(value: unknown): value is Envelope {
  return Value.Check(EnvelopeSchema, value);
}

export function isRunScoreDelta(value: unknown): value is RunScoreDelta {
  return Value.Check(RunScoreDeltaSchema, value);
}

export function isChartFingerprint(value: unknown): value is ChartFingerprint {
  return Value.Check(ChartFingerprintSchema, value);
}
