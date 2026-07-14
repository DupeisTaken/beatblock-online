import type { LobbySnapshot } from '@bbt/protocol';
import { Leaderboard } from './Leaderboard';
import { SignalMark } from './SignalMark';

export function Overlay({
  lobby,
  layout,
  rows,
}: {
  lobby: LobbySnapshot;
  layout: string;
  rows: number;
}) {
  const players = lobby.players.filter((player) => !player.spectator);
  const first = players.find((player) => player.rank === 1) ?? players[0];
  const second = players.find((player) => player.rank === 2) ?? players[1];
  if (layout === 'player-card')
    return (
      <div className="overlay-stage overlay-stage--card">
        {first ? (
          <PlayerCard player={first} song={lobby.chart?.songName ?? 'Waiting for chart'} />
        ) : null}
      </div>
    );
  if (layout === 'versus')
    return (
      <div className="overlay-stage overlay-stage--versus">
        {first ? <PlayerCard player={first} song={lobby.chart?.songName ?? ''} /> : null}
        <div className="versus-slash">VS</div>
        {second ? <PlayerCard player={second} song={lobby.chart?.songName ?? ''} mirrored /> : null}
      </div>
    );
  return (
    <div className={`overlay-stage overlay-stage--${layout}`}>
      <div className="overlay-board">
        <div className="overlay-board__header">
          <SignalMark />
          <div>
            <small>{lobby.name}</small>
            <strong>{lobby.chart?.songName ?? 'Waiting for chart'}</strong>
          </div>
          <span>{lobby.lifecycle.toUpperCase()}</span>
        </div>
        <Leaderboard players={lobby.players} rows={rows} />
      </div>
    </div>
  );
}

function PlayerCard({
  player,
  song,
  mirrored = false,
}: {
  player: LobbySnapshot['players'][number];
  song: string;
  mirrored?: boolean;
}) {
  return (
    <article className={`player-card ${mirrored ? 'player-card--mirrored' : ''}`}>
      <div className="player-card__rail" />
      <div className="player-card__body">
        <small>{song}</small>
        <h2>{player.displayName}</h2>
        <div className="player-card__stats">
          <strong>
            {player.accuracy.toFixed(2)}
            <i>%</i>
          </strong>
          <span>
            {player.totals.combo}
            <small>COMBO</small>
          </span>
          <span>
            {player.totals.misses}
            <small>MISS</small>
          </span>
        </div>
        <div className="player-card__progress">
          <i style={{ transform: `scaleX(${player.progress})` }} />
        </div>
      </div>
    </article>
  );
}
