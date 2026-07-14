# OBS and local API

Open the companion tray console. The URL includes the per-install read-only token. Add one of these as an OBS Browser Source:

```text
http://127.0.0.1:8974/overlay/?layout=player-card&token=<token>
http://127.0.0.1:8974/overlay/?layout=leaderboard&rows=8&token=<token>
http://127.0.0.1:8974/overlay/?layout=versus&token=<token>
http://127.0.0.1:8974/overlay/?layout=caster&token=<token>
```

Use 1280x720, 1920x1080, or 2560x1440. The overlay canvas is transparent and remains active during offline practice.

Local tools can read `GET /v1/state`, `/v1/lobby`, `/v1/players`, and `/v1/run`, or subscribe to `WS /v1/events`. Supply the token as a query parameter. The companion also atomically updates `player_name.txt`, `song_name.txt`, `accuracy.txt`, `combo.txt`, `misses.txt`, `rank.txt`, `lobby_name.txt`, and `state.json`.
