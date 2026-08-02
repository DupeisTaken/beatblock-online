import { Static, Type } from '@sinclair/typebox';
import { Value } from '@sinclair/typebox/value';

export const PROTOCOL_VERSION = 3 as const;
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

export const EnvelopeSchema = Type.Object(
  {
    version: Type.Literal(PROTOCOL_VERSION),
    type: Type.String({ minLength: 1, maxLength: 80 }),
    sequence: Type.Integer({ minimum: 0 }),
    runTimeUs: Type.Integer({ minimum: 0 }),
    runId: Type.Optional(Type.String({ minLength: 1, maxLength: 128 })),
    requestId: Type.Optional(Type.String({ minLength: 1, maxLength: 80 })),
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
  commentatorAccess: Type.Boolean({ default: false }),
});
export type Participant = Static<typeof ParticipantSchema>;
export const PlayerSnapshotSchema = ParticipantSchema;
export type PlayerSnapshot = Participant;

export const RulesSchema = Type.Object(
  {
    rate: Type.Literal(1),
    pauseAllowed: Type.Literal(false),
    retryAllowed: Type.Literal(false),
    forceStart: Type.Boolean(),
  },
  { additionalProperties: false },
);
export type Rules = Static<typeof RulesSchema>;

// These are Beatblock's native per-chart domains. Hosts publish one complete
// policy so clients never merge it with easier local accessibility choices.
const RoomModifierRateSchema = Type.Union(
  Array.from({ length: 46 }, (_, index) => Type.Literal((index + 5) / 10)),
);
export const RoomModifiersSchema = Type.Object(
  {
    // Literal tenths avoid binary floating-point `multipleOf: 0.1` validators
    // rejecting valid values such as 1.7.
    rate: RoomModifierRateSchema,
    vfx: Type.Union([Type.Literal('full'), Type.Literal('decreased'), Type.Literal('none')]),
    taps: Type.Union([
      Type.Literal('default'),
      Type.Literal('lenient'),
      Type.Literal('strict'),
      Type.Literal('auto'),
    ]),
    sides: Type.Union([Type.Literal('default'), Type.Literal('lenient'), Type.Literal('auto')]),
    barelies: Type.Union([
      Type.Literal('default'),
      Type.Literal('lenient'),
      Type.Literal('strict'),
    ]),
    restartOn: Type.Union([Type.Literal('none'), Type.Literal('miss'), Type.Literal('barely')]),
  },
  { additionalProperties: false },
);
export type RoomModifiers = Static<typeof RoomModifiersSchema>;

export const RoomSnapshotSchema = Type.Object({
  id: Type.String({ minLength: 1 }),
  name: Type.String({ minLength: 1, maxLength: 80 }),
  hostSessionId: Type.String({ minLength: 1 }),
  lifecycle: RoomLifecycleSchema,
  admissionMode: Type.Union([Type.Literal('password_only'), Type.Literal('host_approval')]),
  allowChartTransfers: Type.Boolean({ default: true }),
  // Optional for protocol-v3 compatibility. This policy requests an offer
  // after local matching fails; it never grants transfer/install consent.
  autoRequestChartTransfers: Type.Optional(Type.Boolean({ default: false })),
  // Optional keeps protocol-v3 peers compatible; runtimes default omission to
  // strict competitive checking.
  validityChecksEnabled: Type.Optional(Type.Boolean({ default: true })),
  // Optional preserves protocol-v3 decoding. Hosts default omission to exact
  // Beatblock build matching and may relax it only before a race.
  requireSameGameBuild: Type.Optional(Type.Boolean({ default: true })),
  // Optional for historical protocol-v3 snapshots; current peers advertise
  // enforcement support during authenticated room setup.
  modifiers: Type.Optional(RoomModifiersSchema),
  chart: Type.Optional(ChartLockSchema),
  // MAX_PLAYERS already includes the room host.
  participants: Type.Array(ParticipantSchema, { maxItems: MAX_PLAYERS + MAX_SPECTATORS }),
  scheduledStartTimeMs: Type.Optional(Type.Integer({ minimum: 0 })),
  forceStart: Type.Boolean(),
  setlist: Type.Array(
    Type.Object({ id: Type.String(), chart: ChartLockSchema, completed: Type.Boolean() }),
  ),
  currentSetlistIndex: Type.Optional(Type.Union([Type.Integer({ minimum: 0 }), Type.Null()])),
  createdAtMs: Type.Integer({ minimum: 0 }),
  updatedAtMs: Type.Integer({ minimum: 0 }),
});
export type RoomSnapshot = Static<typeof RoomSnapshotSchema>;
export const LobbySnapshotSchema = RoomSnapshotSchema;
export type LobbySnapshot = RoomSnapshot;

export const BroadcastSlotPlanSchema = Type.Object(
  {
    id: Type.Union([Type.Literal('A'), Type.Literal('B'), Type.Literal('C'), Type.Literal('D')]),
    participantId: Type.Optional(Type.String({ minLength: 1 })),
    participantName: Type.Optional(Type.String({ minLength: 1, maxLength: 48 })),
    renderSourceId: Type.Optional(Type.Integer({ minimum: 1 })),
    mode: Type.Union([Type.Literal('clean'), Type.Literal('full')]),
    width: Type.Integer({ minimum: 320, maximum: 1920 }),
    height: Type.Integer({ minimum: 180, maximum: 1080 }),
    fps: Type.Union([Type.Literal(30), Type.Literal(60)]),
    delayMs: Type.Integer({ minimum: 250, maximum: 1500 }),
    featured: Type.Boolean(),
    active: Type.Boolean(),
  },
  { additionalProperties: false },
);
export type BroadcastSlotPlan = Static<typeof BroadcastSlotPlanSchema>;

export const AudioIsolationStateSchema = Type.Object(
  {
    status: Type.Union([
      Type.Literal('pending'),
      Type.Literal('muted'),
      Type.Literal('disabled'),
      Type.Literal('warning'),
    ]),
    muted: Type.Boolean(),
    error: Type.Optional(Type.String({ maxLength: 512 })),
  },
  { additionalProperties: false },
);
export type AudioIsolationState = Static<typeof AudioIsolationStateSchema>;

export const BroadcastPlanSchema = Type.Object(
  {
    revision: Type.Integer({ minimum: 0 }),
    updatedAtMs: Type.Integer({ minimum: 0 }),
    slots: Type.Array(BroadcastSlotPlanSchema, {
      minItems: MAX_RENDER_STREAMS,
      maxItems: MAX_RENDER_STREAMS,
    }),
    // Optional fields preserve protocol-v3 compatibility with beta.4 peers.
    autoplayAudioEnabled: Type.Optional(Type.Boolean()),
    autoplayClockSlot: Type.Optional(
      Type.Union([Type.Literal('A'), Type.Literal('B'), Type.Literal('C'), Type.Literal('D')]),
    ),
  },
  { additionalProperties: false },
);
export type BroadcastPlan = Static<typeof BroadcastPlanSchema>;

export const CommentatorMirrorStatusSchema = Type.Object(
  {
    enabled: Type.Boolean(),
    healthySlots: Type.Integer({ minimum: 0, maximum: MAX_RENDER_STREAMS }),
    error: Type.Optional(Type.String({ maxLength: 160 })),
    updatedAtMs: Type.Integer({ minimum: 0 }),
  },
  { additionalProperties: false },
);
export type CommentatorMirrorStatus = Static<typeof CommentatorMirrorStatusSchema>;

export const ChartTransferOfferSchema = Type.Object(
  {
    requestId: Type.String({ minLength: 1, maxLength: 80 }),
    name: Type.String({ minLength: 1, maxLength: 256 }),
    size: Type.Integer({ minimum: 1, maximum: 1024 * 1024 * 1024 }),
    archiveSha256: Type.String({ pattern: '^[a-f0-9]{64}$' }),
    chartHash: Type.String({ pattern: '^[a-f0-9]{64}$' }),
    containsExecutableContent: Type.Boolean(),
  },
  { additionalProperties: false },
);
export type ChartTransferOffer = Static<typeof ChartTransferOfferSchema>;

export const RunScoreDeltaSchema = Type.Object({
  runSequence: Type.Integer({ minimum: 0 }),
  progress: Type.Number({ minimum: 0, maximum: 1 }),
  beat: Type.Number(),
  songTimeMs: Type.Integer(),
  totals: ScoreTotalsSchema,
});
export type RunScoreDelta = Static<typeof RunScoreDeltaSchema>;

export const RunStartedSchema = Type.Object(
  {
    lobbyId: Type.String({ minLength: 1 }),
    runId: Type.String({ minLength: 1, maxLength: 128 }),
    maxHits: Type.Integer({ minimum: 1 }),
    chartHash: Type.Optional(Type.String({ pattern: '^[a-f0-9]{64}$' })),
    variant: Type.Optional(Type.String({ maxLength: 128 })),
  },
  { additionalProperties: false },
);
export type RunStarted = Static<typeof RunStartedSchema>;

export const RunInvalidSchema = Type.Object(
  {
    lobbyId: Type.String({ minLength: 1 }),
    runId: Type.String({ minLength: 1, maxLength: 128 }),
    reason: Type.String({ minLength: 1, maxLength: 512 }),
    dnf: Type.Boolean(),
  },
  { additionalProperties: false },
);
export type RunInvalid = Static<typeof RunInvalidSchema>;

export const RunFinishedSchema = Type.Object(
  {
    lobbyId: Type.String({ minLength: 1 }),
    runId: Type.String({ minLength: 1, maxLength: 128 }),
    quit: Type.Optional(Type.Boolean()),
  },
  { additionalProperties: false },
);
export type RunFinished = Static<typeof RunFinishedSchema>;

export const ValidityChecksCommandSchema = Type.Object(
  {
    requestId: Type.String({ minLength: 1, maxLength: 80 }),
    enabled: Type.Boolean(),
  },
  { additionalProperties: false },
);
export type ValidityChecksCommand = Static<typeof ValidityChecksCommandSchema>;

export const ClientHelloSchema = Type.Object({
  instanceId: Type.String({ minLength: 1, maxLength: 96 }),
  clientVersion: Type.String({ minLength: 1 }),
  gameVersion: Type.String({ minLength: 1, maxLength: 160 }),
  gameBuildId: Type.String({ minLength: 7, maxLength: 80 }),
  gameBuildSource: Type.Union([
    Type.Literal('displayed_build_hash'),
    Type.Literal('displayed_version_digest'),
    Type.Literal('game_content_digest'),
  ]),
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

function compareUnicodeScalars(left: string, right: string): number {
  // Rust String::cmp orders UTF-8 scalar values. JavaScript's native string
  // comparison orders UTF-16 code units, which differs for supplementary-plane
  // names (for example emoji) versus late-BMP characters.
  let leftIndex = 0;
  let rightIndex = 0;
  while (leftIndex < left.length && rightIndex < right.length) {
    const leftScalar = left.codePointAt(leftIndex)!;
    const rightScalar = right.codePointAt(rightIndex)!;
    if (leftScalar !== rightScalar) {
      return leftScalar - rightScalar;
    }
    leftIndex += leftScalar > 0xffff ? 2 : 1;
    rightIndex += rightScalar > 0xffff ? 2 : 1;
  }
  return left.length - right.length;
}

export function rankPlayers(players: Participant[]): Participant[] {
  const validityOrder: Record<RunValidity, number> = {
    valid: 3,
    pending: 2,
    invalid: 1,
    dnf: 0,
  };
  const competitors = players
    .filter((player) => player.admitted && player.role !== 'spectator')
    .slice()
    .sort((a, b) => {
      if (validityOrder[b.validity] !== validityOrder[a.validity]) {
        return validityOrder[b.validity] - validityOrder[a.validity];
      }
      if (b.accuracy !== a.accuracy) return b.accuracy - a.accuracy;
      if (b.progress !== a.progress) return b.progress - a.progress;
      if (b.totals.maxCombo !== a.totals.maxCombo) return b.totals.maxCombo - a.totals.maxCombo;
      return compareUnicodeScalars(a.displayName, b.displayName);
    });
  const ranks = new Map(competitors.map((player, index) => [player.sessionId, index + 1]));
  return players.map((player) => {
    // Authoritative room ranking clears stale ranks before assigning the next order.
    const next = { ...player };
    delete next.rank;
    const rank = ranks.get(player.sessionId);
    return rank === undefined ? next : { ...next, rank };
  });
}

export function envelope<T>(
  type: string,
  sequence: number,
  payload: T,
  runId?: string,
  requestId?: string,
): Envelope<T> {
  return {
    version: PROTOCOL_VERSION,
    type,
    sequence,
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
export const isRunScoreDelta = (value: unknown): value is RunScoreDelta =>
  Value.Check(RunScoreDeltaSchema, value);
export const isChartFingerprint = (value: unknown): value is ChartFingerprint =>
  Value.Check(ChartLockSchema, value);
