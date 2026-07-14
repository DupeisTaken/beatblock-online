import { useState, type FormEvent } from 'react';
import { ExternalLink, Link2, MonitorUp, RadioTower } from 'lucide-react';
import type { LobbySnapshot } from '@bbt/protocol';
import { localEndpoint } from '../hooks/useLiveFeed';

async function command(path: string, method: 'POST' | 'PUT', body: unknown) {
  const response = await fetch(localEndpoint(path), {
    method,
    headers: { 'content-type': 'application/json' },
    body: JSON.stringify(body),
  });
  const value = (await response.json().catch(() => ({}))) as { error?: string; url?: string };
  if (!response.ok) throw new Error(value.error ?? `Request failed (${response.status})`);
  return value;
}

export function ControlDeck({ lobby }: { lobby: LobbySnapshot }) {
  const [message, setMessage] = useState('Lobby controls are available inside Beatblock');
  const [busy, setBusy] = useState(false);
  if (!new URLSearchParams(location.search).has('token')) return null;

  const run = async (task: () => Promise<unknown>, success: string) => {
    setBusy(true);
    try {
      await task();
      setMessage(success);
    } catch (error) {
      setMessage(error instanceof Error ? error.message : 'Command failed');
    } finally {
      setBusy(false);
    }
  };
  const overlay = (layout: string) => {
    const url = new URL('/overlay/', location.origin);
    url.searchParams.set('layout', layout);
    const token = new URLSearchParams(location.search).get('token');
    if (token) url.searchParams.set('token', token);
    window.open(url, '_blank', 'noopener,noreferrer');
  };

  return (
    <section className="control-deck">
      <div className="control-deck__heading">
        <div>
          <span>COMPANION CONSOLE</span>
          <h2>Setup and broadcast</h2>
        </div>
        <output>{message}</output>
      </div>
      <div className="control-deck__grid">
        <RedeemForm disabled={busy} run={run} />
        <div className="command-block">
          <span className="command-block__index">02 / IN GAME</span>
          <h3>Competition controls</h3>
          <p>
            Open <strong>Online</strong> from Beatblock&apos;s main menu to create or join a lobby,
            select and verify the chart, ready up, and start the race.
          </p>
          <p>The browser is no longer required during normal play.</p>
        </div>
        <div className="command-block">
          <span className="command-block__index">03 / OBS</span>
          <h3>Overlay sources</h3>
          <div className="command-row">
            <button disabled={busy} onClick={() => overlay('player-card')}>
              <MonitorUp /> Player card
            </button>
            <button disabled={busy} onClick={() => overlay('leaderboard')}>
              <ExternalLink /> Leaderboard
            </button>
          </div>
          <div className="command-row">
            <button disabled={busy} onClick={() => overlay('versus')}>
              <ExternalLink /> Versus
            </button>
            <button disabled={busy} onClick={() => overlay('caster')}>
              <ExternalLink /> Caster
            </button>
          </div>
        </div>
        <div className="command-block">
          <span className="command-block__index">04 / SPECTATE</span>
          <h3>Authenticated caster view</h3>
          <p>Create a one-time browser handoff for the lobby currently joined in Beatblock.</p>
          <button
            disabled={busy || lobby.id === 'offline'}
            onClick={() =>
              void run(async () => {
                const value = await command('/v1/spectate', 'POST', { lobbyId: lobby.id });
                if (value.url) window.open(value.url, '_blank', 'noopener,noreferrer');
              }, 'Spectator handoff opened')
            }
          >
            <RadioTower /> Open spectator view
          </button>
        </div>
      </div>
    </section>
  );
}

type Run = (task: () => Promise<unknown>, success: string) => Promise<void>;

function RedeemForm({ disabled, run }: { disabled: boolean; run: Run }) {
  const submit = (event: FormEvent<HTMLFormElement>) => {
    event.preventDefault();
    const data = new FormData(event.currentTarget);
    void run(
      () =>
        command('/v1/redeem', 'POST', {
          instanceUrl: data.get('instance'),
          inviteCode: data.get('invite'),
          displayName: data.get('name'),
        }),
      'Account connected. Continue inside Beatblock.',
    );
  };
  return (
    <form className="command-block" onSubmit={submit}>
      <span className="command-block__index">01 / IDENTITY</span>
      <h3>Redeem invite</h3>
      <label>
        Instance URL
        <input name="instance" type="url" placeholder="https://together.example.com" required />
      </label>
      <div className="command-pair">
        <label>
          Invite
          <input name="invite" placeholder="BBT-XXXX-XXXX" required />
        </label>
        <label>
          Display name
          <input name="name" minLength={3} maxLength={32} required />
        </label>
      </div>
      <button disabled={disabled}>
        <Link2 /> Connect instance
      </button>
    </form>
  );
}
