import { memo } from 'react';
import type { PlayerSnapshot } from '@bbt/protocol';

function LeaderboardRowComponent({
  player,
  featured = false,
}: {
  player: PlayerSnapshot;
  featured?: boolean;
}) {
  const invalid = player.validity === 'invalid' || player.validity === 'dnf';
  return (
    <div
      className={`leader-row ${featured ? 'leader-row--featured' : ''} ${invalid ? 'leader-row--invalid' : ''}`}
    >
      <div className="leader-row__rank">
        {invalid ? '—' : String(player.rank ?? 0).padStart(2, '0')}
      </div>
      <div className="leader-row__identity">
        <strong>{player.displayName}</strong>
        <span>
          {invalid
            ? (player.invalidReason ?? player.validity)
            : player.connected
              ? `${player.totals.combo} combo`
              : 'signal lost'}
        </span>
      </div>
      <div
        className="leader-row__progress"
        aria-label={`${Math.round(player.progress * 100)} percent complete`}
      >
        <i style={{ transform: `scaleX(${player.progress})` }} />
      </div>
      <div className="leader-row__score">
        <strong>{player.accuracy.toFixed(2)}</strong>
        <span>%</span>
      </div>
      <div className="leader-row__misses">
        <span>M</span>
        {player.totals.misses}
      </div>
    </div>
  );
}

export const LeaderboardRow = memo(LeaderboardRowComponent);

export function Leaderboard({
  players,
  rows = 16,
  featuredId,
}: {
  players: PlayerSnapshot[];
  rows?: number;
  featuredId?: string;
}) {
  return (
    <div className="leaderboard">
      {players
        .filter((player) => !player.spectator)
        .slice(0, rows)
        .map((player) => (
          <LeaderboardRow
            key={player.userId}
            player={player}
            featured={player.userId === featuredId}
          />
        ))}
    </div>
  );
}
