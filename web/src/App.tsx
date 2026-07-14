import { Dashboard } from './components/Dashboard';
import { Overlay } from './components/Overlay';
import { useLiveFeed } from './hooks/useLiveFeed';

export function App() {
  const { lobby, status, lastEventMs } = useLiveFeed();
  const params = new URLSearchParams(location.search);
  const path = location.pathname;
  if (path.startsWith('/overlay'))
    return (
      <Overlay
        lobby={lobby}
        layout={params.get('layout') ?? 'leaderboard'}
        rows={Math.min(16, Math.max(2, Number(params.get('rows') ?? 8)))}
      />
    );
  return <Dashboard lobby={lobby} status={status} lastEventMs={lastEventMs} />;
}
