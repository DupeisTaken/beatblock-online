import { describe, expect, it } from 'vitest';
import {
  calculateAccuracy,
  BroadcastPlanSchema,
  ChartTransferOfferSchema,
  ClientHelloSchema,
  decodeRenderDatagram,
  EMPTY_TOTALS,
  encodeRenderDatagram,
  isEnvelope,
  PROTOCOL_VERSION,
  rankPlayers,
  RoomSnapshotSchema,
  RunFinishedSchema,
  RunInvalidSchema,
  RunStartedSchema,
  ValidityChecksCommandSchema,
} from '../src/index.js';
import { Value } from '@sinclair/typebox/value';

describe('Beatblock score derivation', () => {
  it('keeps pre-race room policies optional for protocol-v3 compatibility', () => {
    const room = {
      id: 'room',
      name: 'Room',
      hostSessionId: 'host',
      lifecycle: 'forming',
      admissionMode: 'host_approval',
      allowChartTransfers: true,
      participants: [],
      forceStart: false,
      setlist: [],
      currentSetlistIndex: null,
      createdAtMs: 1,
      updatedAtMs: 1,
    };
    expect(Value.Check(RoomSnapshotSchema, room)).toBe(true);
    expect(Value.Check(RoomSnapshotSchema, { ...room, validityChecksEnabled: false })).toBe(true);
    expect(Value.Check(RoomSnapshotSchema, { ...room, autoRequestChartTransfers: true })).toBe(
      true,
    );
    expect(Value.Check(RoomSnapshotSchema, { ...room, requireSameGameBuild: false })).toBe(true);
  });

  it('accepts Rust-shaped snapshots with absent optional result fields', () => {
    const room = {
      id: 'room',
      name: 'Room',
      hostSessionId: 'host',
      lifecycle: 'playing',
      admissionMode: 'host_approval',
      allowChartTransfers: true,
      validityChecksEnabled: true,
      participants: [
        {
          sessionId: 'host',
          displayName: 'Host',
          role: 'host',
          admitted: true,
          connected: true,
          ready: true,
          verified: true,
          progress: 0,
          accuracy: 100,
          setTotal: 0,
          totals: { ...EMPTY_TOTALS, maxHits: 100 },
          validity: 'pending',
          commentatorAccess: false,
        },
      ],
      scheduledStartTimeMs: 1000,
      forceStart: false,
      setlist: [],
      createdAtMs: 1,
      updatedAtMs: 1,
    };
    expect(Value.Check(RoomSnapshotSchema, room)).toBe(true);
  });

  it('validates run lifecycle and validity-policy payloads strictly', () => {
    expect(
      Value.Check(RunStartedSchema, {
        lobbyId: 'room',
        runId: 'run-1',
        maxHits: 100,
        chartHash: 'a'.repeat(64),
        variant: 'Hard',
      }),
    ).toBe(true);
    expect(
      Value.Check(RunInvalidSchema, {
        lobbyId: 'room',
        runId: 'run-1',
        reason: 'Sequence gap',
        dnf: false,
      }),
    ).toBe(true);
    expect(Value.Check(RunInvalidSchema, { runId: 'run-1', reason: '', dnf: 'no' })).toBe(false);
    expect(Value.Check(RunFinishedSchema, { lobbyId: 'room', runId: 'run-1', quit: false })).toBe(
      true,
    );
    expect(
      Value.Check(ValidityChecksCommandSchema, { requestId: 'request-1', enabled: false }),
    ).toBe(true);
    expect(Value.Check(ValidityChecksCommandSchema, { requestId: 'request-1' })).toBe(false);
  });

  it('matches perfect, barely, and miss scoring', () => {
    expect(calculateAccuracy({ currentMaxHits: 100, misses: 0, barelies: 0 })).toBe(100);
    expect(calculateAccuracy({ currentMaxHits: 100, misses: 0, barelies: 1 })).toBe(99.75);
    expect(calculateAccuracy({ currentMaxHits: 100, misses: 1, barelies: 0 })).toBe(99);
  });

  it('floors to two decimal places', () => {
    expect(calculateAccuracy({ currentMaxHits: 3, misses: 1, barelies: 0 })).toBe(66.66);
  });

  it('ranks by accuracy, progress, max combo, then name', () => {
    const players = ['B', 'A'].map((displayName, index) => ({
      sessionId: String(index),
      displayName,
      role: 'player' as const,
      admitted: true,
      connected: true,
      ready: true,
      verified: true,
      progress: 0.5,
      accuracy: 99,
      setTotal: 0,
      totals: { ...EMPTY_TOTALS, maxCombo: 10 },
      validity: 'valid' as const,
    }));
    expect(rankPlayers(players).find((player) => player.displayName === 'A')?.rank).toBe(1);
  });

  it('matches authoritative admission and validity ranking', () => {
    const player = (
      sessionId: string,
      validity: 'valid' | 'pending' | 'invalid' | 'dnf',
      accuracy: number,
      admitted = true,
    ) => ({
      sessionId,
      displayName: sessionId,
      role: 'player' as const,
      admitted,
      connected: true,
      ready: true,
      verified: true,
      progress: 1,
      accuracy,
      setTotal: 0,
      totals: { ...EMPTY_TOTALS },
      validity,
      rank: 99,
      commentatorAccess: false,
    });
    const ranked = rankPlayers([
      player('valid', 'valid', 50),
      player('pending', 'pending', 100),
      player('invalid', 'invalid', 100),
      player('dnf', 'dnf', 100),
      player('unadmitted', 'valid', 100, false),
    ]);

    expect(ranked.map(({ sessionId, rank }) => [sessionId, rank])).toEqual([
      ['valid', 1],
      ['pending', 2],
      ['invalid', 3],
      ['dnf', 4],
      ['unadmitted', undefined],
    ]);
  });

  it('uses the same Unicode scalar tie-break as the Rust room engine', () => {
    const base = {
      role: 'player' as const,
      admitted: true,
      connected: true,
      ready: true,
      verified: true,
      progress: 1,
      accuracy: 100,
      setTotal: 0,
      totals: { ...EMPTY_TOTALS },
      validity: 'valid' as const,
      commentatorAccess: false,
    };
    const ranked = rankPlayers([
      { ...base, sessionId: 'supplementary', displayName: '\u{10000}' },
      { ...base, sessionId: 'bmp', displayName: '\u{e000}' },
    ]);

    expect(ranked.find(({ sessionId }) => sessionId === 'bmp')?.rank).toBe(1);
    expect(ranked.find(({ sessionId }) => sessionId === 'supplementary')?.rank).toBe(2);
  });

  it('round-trips the packed 60 Hz renderer datagram', () => {
    const encoded = encodeRenderDatagram({
      sessionId: 2,
      sequence: 418,
      runTimeUs: 82431000n,
      beat: 42.5,
      paddleAngle: 187.25,
      tapMask: 3,
      flags: 1,
    });
    expect(encoded).toHaveLength(32);
    expect(decodeRenderDatagram(encoded)).toEqual({
      sessionId: 2,
      sequence: 418,
      runTimeUs: 82431000n,
      beat: 42.5,
      paddleAngle: 187.25,
      tapMask: 3,
      flags: 1,
    });
  });

  it('accepts protocol v3 envelopes and explicitly rejects v2', () => {
    const message = {
      version: PROTOCOL_VERSION,
      type: 'room.snapshot',
      sequence: 1,
      runTimeUs: 2,
      payload: {},
    };
    expect(isEnvelope(message)).toBe(true);
    expect(isEnvelope({ ...message, version: 2 })).toBe(false);
  });

  it('requires four health-free slots in an authoritative Broadcast plan', () => {
    const slots = ['A', 'B', 'C', 'D'].map((id, index) => ({
      id,
      mode: 'clean',
      width: 1280,
      height: 720,
      fps: 60,
      delayMs: 500,
      featured: index === 0,
      active: false,
    }));
    expect(Value.Check(BroadcastPlanSchema, { revision: 1, updatedAtMs: 10, slots })).toBe(true);
    expect(
      Value.Check(BroadcastPlanSchema, {
        revision: 2,
        updatedAtMs: 11,
        slots,
        autoplayAudioEnabled: true,
        autoplayClockSlot: 'A',
      }),
    ).toBe(true);
    expect(
      Value.Check(BroadcastPlanSchema, { revision: 1, updatedAtMs: 10, slots: slots.slice(1) }),
    ).toBe(false);
  });

  it('validates the runtime client hello ownership key', () => {
    const hello = {
      instanceId: 'game-123',
      clientVersion: '0.3.0-beta.5',
      gameVersion: '1.7.1a (Early Access)[d40b7083]',
      gameBuildId: 'd40b7083',
      gameBuildSource: 'displayed_build_hash',
      distribution: 'standalone',
      mods: [],
    };
    expect(Value.Check(ClientHelloSchema, hello)).toBe(true);
    expect(Value.Check(ClientHelloSchema, { ...hello, instanceId: undefined })).toBe(false);
    expect(Value.Check(ClientHelloSchema, { ...hello, instanceId: 'x'.repeat(97) })).toBe(false);
  });

  it('validates the chart-transfer offer emitted by the Rust runtime', () => {
    const offer = {
      requestId: 'peer-1-1234',
      name: 'room host',
      size: 1024,
      archiveSha256: 'a'.repeat(64),
      chartHash: 'b'.repeat(64),
      containsExecutableContent: false,
    };
    expect(Value.Check(ChartTransferOfferSchema, offer)).toBe(true);
    expect(Value.Check(ChartTransferOfferSchema, { ...offer, size: 0 })).toBe(false);
    expect(Value.Check(ChartTransferOfferSchema, { ...offer, compressedBytes: offer.size })).toBe(
      false,
    );
  });
});
