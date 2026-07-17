import { mkdir, writeFile } from 'node:fs/promises';
import { resolve } from 'node:path';
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
    PlayerSnapshot: PlayerSnapshotSchema,
    LobbySnapshot: LobbySnapshotSchema,
    RunScoreDelta: RunScoreDeltaSchema,
    ClientHello: ClientHelloSchema,
    BroadcastPlan: BroadcastPlanSchema,
    CommentatorMirrorStatus: CommentatorMirrorStatusSchema,
    ChartTransferOffer: ChartTransferOfferSchema,
  },
};
await writeFile(resolve(output, 'protocol.json'), `${JSON.stringify(schema, null, 2)}\n`);
console.log('Generated protocol/schemas/v3/protocol.json');
