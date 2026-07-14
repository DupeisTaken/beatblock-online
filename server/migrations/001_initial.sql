-- The server runs equivalent idempotent DDL from PostgresStore.migrate().
-- This file is provided for operators who require externally managed migrations.
create table if not exists users (id text primary key, display_name text not null unique, role text not null, disabled boolean not null default false, created_at_ms bigint not null);
create table if not exists invites (id text primary key, code_hash text not null unique, role text not null, max_redemptions integer not null, redemptions integer not null default 0, expires_at_ms bigint, revoked_at_ms bigint, created_at_ms bigint not null);
create table if not exists sessions (id text primary key, user_id text not null references users(id), refresh_hash text not null unique, device_name text not null, expires_at_ms bigint not null, revoked_at_ms bigint, created_at_ms bigint not null);
create table if not exists browser_tickets (hash text primary key, user_id text not null references users(id), expires_at_ms bigint not null, consumed_at_ms bigint);
create table if not exists lobbies (id text primary key, code text not null unique, lifecycle text not null, snapshot jsonb not null, updated_at_ms bigint not null);
create table if not exists run_events (run_id text not null, sequence bigint not null, lobby_id text not null, user_id text not null, received_at_ms bigint not null, envelope jsonb not null, primary key (run_id, sequence));
create table if not exists runs (run_id text primary key, lobby_id text not null, user_id text not null, accuracy double precision not null, progress double precision not null, validity text not null, invalid_reason text, totals jsonb not null, updated_at_ms bigint not null);
create table if not exists audit_log (id bigserial primary key, actor_id text, action text not null, subject_id text, metadata jsonb, created_at_ms bigint not null);
create table if not exists allowed_mods (id text primary key, hash text not null);
