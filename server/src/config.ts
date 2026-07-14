export interface Config {
  host: string;
  port: number;
  publicUrl: string;
  databaseUrl: string;
  tokenSecret: string;
  inviteSecret: string;
  accessTokenTtlSeconds: number;
  refreshTokenTtlDays: number;
  runEventRetentionDays: number;
  allowInsecureHttp: boolean;
  supportedGameBuilds: string[];
}

function required(name: string, fallback?: string): string {
  const value = process.env[name] ?? fallback;
  if (!value) throw new Error(`${name} is required`);
  return value;
}

export function loadConfig(): Config {
  const tokenSecret = required('TOKEN_SECRET', 'development-token-secret-change-before-deploying');
  const inviteSecret = required(
    'INVITE_SECRET',
    'development-invite-secret-change-before-deploying',
  );
  if (
    process.env.NODE_ENV === 'production' &&
    (tokenSecret.startsWith('development-') || inviteSecret.startsWith('development-'))
  ) {
    throw new Error('Production requires unique TOKEN_SECRET and INVITE_SECRET values');
  }
  return {
    host: process.env.HOST ?? '0.0.0.0',
    port: Number(process.env.PORT ?? 8787),
    publicUrl: process.env.PUBLIC_URL ?? 'http://127.0.0.1:8787',
    databaseUrl: process.env.DATABASE_URL ?? 'memory://',
    tokenSecret,
    inviteSecret,
    accessTokenTtlSeconds: Number(process.env.ACCESS_TOKEN_TTL_SECONDS ?? 900),
    refreshTokenTtlDays: Number(process.env.REFRESH_TOKEN_TTL_DAYS ?? 30),
    runEventRetentionDays: Number(process.env.RUN_EVENT_RETENTION_DAYS ?? 30),
    allowInsecureHttp: process.env.ALLOW_INSECURE_HTTP === 'true',
    supportedGameBuilds: (
      process.env.SUPPORTED_GAME_BUILDS ??
      'c91d0853feb12aceb66a821eb5cdffb9c25acf69268bb2cf7451fa42f864de6b'
    )
      .split(',')
      .map((value) => value.trim().toLowerCase())
      .filter(Boolean),
  };
}
