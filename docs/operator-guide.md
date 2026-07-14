# Operator guide

## Deploy

1. Point a DNS record at the host.
2. Copy `.env.example` to `.env` and replace the domain, database password, and both secrets.
3. Run `docker compose --env-file .env -f deploy/compose.yml up -d --build`.
4. Confirm `https://<domain>/health` reports protocol version 1.

Caddy obtains public certificates automatically. For external TLS termination, set `PUBLIC_URL` to the external HTTPS origin and forward WebSocket upgrades for `/api/v1/gateway`.

## Administration

```text
bbtctl invite-create --role player --uses 1 --expires-hours 168
bbtctl invite-create --role organizer --uses 1
bbtctl invite-list
bbtctl invite-revoke <invite-id>
bbtctl user-list
bbtctl user-role <user-id> organizer|operator|player
bbtctl user-disable <user-id>
bbtctl user-enable <user-id>
bbtctl session-reset <user-id>
bbtctl allow-mod <mod-id> <sha256>
bbtctl remove-mod <mod-id>
bbtctl list-mods
bbtctl retention-prune --days 30
bbtctl status
```

Invite codes are shown once. Disabling an account blocks authenticated operations and reconnects. `session-reset` revokes every device refresh credential.

## Backup and recovery

Back up PostgreSQL with `pg_dump` and test restoration before events. Lobby snapshots and ordered run events persist in PostgreSQL; companions resend local journals idempotently after restarts. Missing sequence data invalidates a run.
