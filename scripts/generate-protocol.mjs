import { mkdir, writeFile } from 'node:fs/promises';
import { resolve } from 'node:path';
import { format, resolveConfig } from 'prettier';
import {
  ChartFingerprintSchema,
  ChartTransferOfferSchema,
  ClientHelloSchema,
  BroadcastPlanSchema,
  CommentatorMirrorStatusSchema,
  EnvelopeSchema,
  LobbySnapshotSchema,
  PlayerSnapshotSchema,
  RulesSchema,
  RoomModifiersSchema,
  RunScoreDeltaSchema,
  ScoreTotalsSchema,
} from '../protocol/dist/index.js';

const root = resolve(import.meta.dirname, '..');
const output = resolve(root, 'protocol/schemas/v3');
await mkdir(output, { recursive: true });
const schema = {
  $schema: 'https://json-schema.org/draft/2020-12/schema',
  $id: 'https://beatblock-online.local/protocol/v3/protocol.json',
  title: 'Beatblock Online protocol v3',
  $defs: {
    Envelope: EnvelopeSchema,
    ScoreTotals: ScoreTotalsSchema,
    ChartFingerprint: ChartFingerprintSchema,
    Rules: RulesSchema,
    RoomModifiers: RoomModifiersSchema,
    PlayerSnapshot: PlayerSnapshotSchema,
    LobbySnapshot: LobbySnapshotSchema,
    RunScoreDelta: RunScoreDeltaSchema,
    ClientHello: ClientHelloSchema,
    BroadcastPlan: BroadcastPlanSchema,
    CommentatorMirrorStatus: CommentatorMirrorStatusSchema,
    ChartTransferOffer: ChartTransferOfferSchema,
  },
};
const outputPath = resolve(output, 'protocol.json');
const prettierConfig = (await resolveConfig(outputPath)) ?? {};
const serialized = await format(JSON.stringify(schema), {
  ...prettierConfig,
  parser: 'json',
});
await writeFile(outputPath, serialized);
console.log('Generated protocol/schemas/v3/protocol.json');
