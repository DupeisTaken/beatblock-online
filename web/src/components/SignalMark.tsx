export function SignalMark({ compact = false }: { compact?: boolean }) {
  return (
    <div
      className={`signal-mark ${compact ? 'signal-mark--compact' : ''}`}
      aria-label="Beatblock Together"
    >
      <span className="signal-mark__orbit" aria-hidden="true">
        <i />
        <i />
        <i />
      </span>
      {compact ? null : (
        <span>
          <b>BEATBLOCK</b>
          <em>TOGETHER</em>
        </span>
      )}
    </div>
  );
}
