import { startTransition, useEffect, useRef, useState } from 'react';
import type { Envelope, LobbySnapshot } from '@bbt/protocol';
import { demoLobby, offlineLobby, stressDemoLobby } from '../demo';

export type ConnectionStatus = 'connecting' | 'live' | 'stale' | 'demo';

export function localEndpoint(path: string): string {
  const params = new URLSearchParams(location.search);
  const base = params.get('api') ?? location.origin;
  const token = params.get('token');
  const url = new URL(path, base);
  if (token) url.searchParams.set('token', token);
  return url.toString();
}

export function useLiveFeed(): {
  lobby: LobbySnapshot;
  status: ConnectionStatus;
  lastEventMs: number;
} {
  const params = new URLSearchParams(location.search);
  const demo = params.get('demo') === '1';
  const [lobby, setLobby] = useState<LobbySnapshot>(
    demo ? (params.get('stress') === '1' ? stressDemoLobby : demoLobby) : offlineLobby,
  );
  const [status, setStatus] = useState<ConnectionStatus>(demo ? 'demo' : 'connecting');
  const [lastEventMs, setLastEventMs] = useState(Date.now());
  const pending = useRef<LobbySnapshot | undefined>(undefined);
  const frame = useRef<number | undefined>(undefined);

  useEffect(() => {
    if (demo) return;
    let active = true;
    let socket: WebSocket | undefined;
    const flush = () => {
      frame.current = undefined;
      if (!pending.current) return;
      const next = pending.current;
      pending.current = undefined;
      startTransition(() => setLobby(next));
      setLastEventMs(Date.now());
    };
    const queue = (next: LobbySnapshot) => {
      pending.current = next;
      frame.current ??= requestAnimationFrame(flush);
    };
    const onMessage = (event: MessageEvent) => {
      try {
        const message = JSON.parse(String(event.data)) as Envelope;
        if (message.type === 'lobby.snapshot') queue(message.payload as LobbySnapshot);
      } catch {
        // Malformed messages are ignored and surfaced in service logs.
      }
    };

    const connectLocal = async () => {
      const response = await fetch(localEndpoint('/v1/lobby'));
      if (!response.ok) throw new Error('Local companion is unavailable');
      queue((await response.json()) as LobbySnapshot);
      socket = new WebSocket(localEndpoint('/v1/events').replace(/^http/, 'ws'));
      socket.addEventListener('open', () => setStatus('live'));
      socket.addEventListener('message', onMessage);
      socket.addEventListener('close', () => active && setStatus('stale'));
    };

    const connectRemote = async (api: string, ticket: string | null, lobbyId: string) => {
      let accessToken = sessionStorage.getItem('bbt-browser-access-token');
      if (ticket) {
        const exchange = await fetch(new URL('/api/v1/auth/browser-exchange', api), {
          method: 'POST',
          headers: { 'content-type': 'application/json' },
          body: JSON.stringify({ ticket }),
        });
        if (!exchange.ok) throw new Error('Spectator handoff is invalid or expired');
        accessToken = ((await exchange.json()) as { accessToken: string }).accessToken;
        sessionStorage.setItem('bbt-browser-access-token', accessToken);
        history.replaceState(
          null,
          '',
          `${location.pathname}?remote=1&api=${encodeURIComponent(api)}&lobby=${encodeURIComponent(lobbyId)}`,
        );
      }
      if (!accessToken) throw new Error('Spectator session is unavailable');
      const snapshot = await fetch(new URL(`/api/v1/lobbies/${encodeURIComponent(lobbyId)}`, api), {
        headers: { authorization: `Bearer ${accessToken}` },
      });
      if (!snapshot.ok) throw new Error('Lobby is unavailable');
      queue((await snapshot.json()) as LobbySnapshot);
      const gateway = new URL('/api/v1/gateway', api);
      gateway.protocol = gateway.protocol === 'https:' ? 'wss:' : 'ws:';
      gateway.searchParams.set('access_token', accessToken);
      socket = new WebSocket(gateway);
      socket.addEventListener('open', () => {
        socket?.send(
          JSON.stringify({
            version: 1,
            type: 'lobby.subscribe',
            sequence: 0,
            timestampMs: Date.now(),
            payload: { lobbyId },
          }),
        );
        setStatus('live');
      });
      socket.addEventListener('message', onMessage);
      socket.addEventListener('close', () => active && setStatus('stale'));
    };

    const ticket = params.get('ticket');
    const lobbyId = params.get('lobby');
    const api = params.get('api');
    const task =
      lobbyId && api && (ticket || params.get('remote') === '1')
        ? connectRemote(api, ticket, lobbyId)
        : connectLocal();
    void task.catch(() => {
      if (active) setStatus('stale');
    });
    return () => {
      active = false;
      if (frame.current) cancelAnimationFrame(frame.current);
      socket?.close();
    };
  }, [demo]);

  return { lobby, status, lastEventMs };
}
