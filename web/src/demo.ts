import { DEFAULT_RULES, EMPTY_TOTALS, rankPlayers, type LobbySnapshot } from '@bbt/protocol';

const rows = [
  ['MiraFlux', 99.75, 0.82, 714, 0, 1],
  ['Sammy', 99.62, 0.84, 682, 0, 2],
  ['Kite', 98.91, 0.79, 405, 1, 1],
  ['NOCTURNE', 97.5, 0.86, 328, 2, 2],
  ['Bliv', 96.84, 0.77, 276, 3, 1],
] as const;

export const offlineLobby: LobbySnapshot = {
  id: 'offline',
  code: 'LOCAL',
  name: 'Beatblock companion',
  organizerId: 'local',
  lifecycle: 'forming',
  rules: DEFAULT_RULES,
  players: [],
  createdAtMs: Date.now(),
  updatedAtMs: Date.now(),
};

export const demoLobby: LobbySnapshot = {
  id: 'lobby_demo',
  code: 'R7M4QX',
  name: 'Tuesday Signal / Finals',
  organizerId: 'user_1',
  lifecycle: 'playing',
  chart: {
    algorithm: 'sha256-canonical-package-v1',
    hash: 'a'.repeat(64),
    packageName: '3bitbebop.zip',
    songName: '3-Bit Bebop',
    variant: 'Hard',
    expectedMaxHits: 922,
  },
  rules: DEFAULT_RULES,
  players: rankPlayers(
    rows.map(([displayName, accuracy, progress, combo, misses, barelies], index) => ({
      userId: `user_${index + 1}`,
      displayName,
      connected: index !== 4,
      ready: true,
      spectator: false,
      progress,
      accuracy,
      totals: {
        ...EMPTY_TOTALS,
        hits: Math.round(progress * 922) - misses,
        misses,
        barelies,
        combo,
        maxCombo: combo + index * 44,
        currentMaxHits: Math.round(progress * 922),
        maxHits: 922,
      },
      validity: index === 4 ? ('dnf' as const) : ('valid' as const),
      ...(index === 4 ? { invalidReason: 'Connection journal incomplete' } : {}),
    })),
  ),
  scheduledStartTimeMs: Date.now() - 87_000,
  createdAtMs: Date.now() - 600_000,
  updatedAtMs: Date.now(),
};

export const stressDemoLobby: LobbySnapshot = {
  ...demoLobby,
  id: 'lobby_stress',
  name: 'International Invitational / Sixteen Player Final',
  players: rankPlayers(
    Array.from({ length: 16 }, (_, index) => ({
      userId: `stress_${index}`,
      displayName:
        index === 3
          ? 'VERY-LONG-PLAYER-NAME-ALPHA'
          : `Competitor ${String(index + 1).padStart(2, '0')}`,
      connected: index !== 14,
      ready: true,
      spectator: false,
      progress: 0.92 - index * 0.013,
      accuracy: index < 2 ? 99.75 : 99.5 - index * 0.17,
      totals: {
        ...EMPTY_TOTALS,
        hits: 800 - index * 10,
        misses: Math.floor(index / 4),
        barelies: index % 3,
        combo: 600 - index * 22,
        maxCombo: 620 - index * 20,
        currentMaxHits: 840 - index * 12,
        maxHits: 922,
      },
      validity: index === 15 ? ('dnf' as const) : ('valid' as const),
      ...(index === 15 ? { invalidReason: 'Disconnected / journal incomplete' } : {}),
    })),
  ),
};
