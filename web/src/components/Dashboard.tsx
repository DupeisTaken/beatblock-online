import { Activity, Clock3, Copy, Radio, ShieldCheck, UsersRound } from 'lucide-react';
import type { LobbySnapshot } from '@bbt/protocol';
import type { ConnectionStatus } from '../hooks/useLiveFeed';
import { Leaderboard } from './Leaderboard';
import { SignalMark } from './SignalMark';
import { ControlDeck } from './ControlDeck';

export function Dashboard({
  lobby,
  status,
  lastEventMs,
}: {
  lobby: LobbySnapshot;
  status: ConnectionStatus;
  lastEventMs: number;
}) {
  const competitors = lobby.players.filter((player) => !player.spectator);
  const connected = competitors.filter((player) => player.connected).length;
  return (
    <main className="console-shell">
      <header className="console-header">
        <SignalMark />
        <div className="console-header__status">
          <span className={`live-dot live-dot--${status}`} />
          <div>
            <small>SIGNAL</small>
            <strong>{status.toUpperCase()}</strong>
          </div>
        </div>
      </header>

      <section className="hero-strip">
        <div className="eyebrow">
          <Radio size={15} /> COMPANION / BROADCAST / {lobby.lifecycle.replace('_', ' ')}
        </div>
        <h1>{lobby.name}</h1>
        <div className="hero-strip__meta">
          <span>
            <b>{lobby.chart?.songName ?? 'Chart pending'}</b> / {lobby.chart?.variant ?? '—'}
          </span>
          <button
            type="button"
            onClick={() => void navigator.clipboard?.writeText(lobby.code)}
            aria-label="Copy lobby code"
          >
            <Copy size={16} /> {lobby.code}
          </button>
        </div>
      </section>

      <section className="metric-grid" aria-label="Match metrics">
        <Metric
          icon={<UsersRound />}
          label="Grid"
          value={`${connected}/${competitors.length}`}
          detail="players online"
        />
        <Metric icon={<Activity />} label="Pulse" value="10.0" detail="updates / sec" />
        <Metric
          icon={<Clock3 />}
          label="Age"
          value={`${Math.max(0, Math.round((Date.now() - lastEventMs) / 100) / 10)}s`}
          detail="since packet"
        />
        <Metric
          icon={<ShieldCheck />}
          label="Rules"
          value="LOCK"
          detail="competitive defaults"
          accent
        />
      </section>

      <ControlDeck lobby={lobby} />

      <section className="board-panel">
        <div className="panel-heading">
          <div>
            <span>LIVE CLASSIFICATION</span>
            <h2>Race telemetry</h2>
          </div>
          <div className="hash-chip">
            CHART <b>{lobby.chart?.hash.slice(0, 8).toUpperCase() ?? 'PENDING'}</b>
          </div>
        </div>
        <Leaderboard players={lobby.players} />
      </section>

      <footer className="console-footer">
        <span>BBT / PROTOCOL 01</span>
        <span>INVITE-ONLY INSTANCE</span>
        <span>{new Date(lobby.updatedAtMs).toLocaleTimeString([], { hour12: false })}</span>
      </footer>
    </main>
  );
}

function Metric({
  icon,
  label,
  value,
  detail,
  accent = false,
}: {
  icon: React.ReactNode;
  label: string;
  value: string;
  detail: string;
  accent?: boolean;
}) {
  return (
    <article className={`metric ${accent ? 'metric--accent' : ''}`}>
      <div className="metric__icon">{icon}</div>
      <div>
        <span>{label}</span>
        <strong>{value}</strong>
        <small>{detail}</small>
      </div>
    </article>
  );
}
