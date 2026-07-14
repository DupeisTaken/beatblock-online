FROM node:22-bookworm-slim AS build
RUN corepack enable
WORKDIR /app
COPY package.json pnpm-lock.yaml pnpm-workspace.yaml tsconfig.base.json .prettierrc.json ./
COPY protocol/package.json protocol/tsconfig.json ./protocol/
COPY server/package.json server/tsconfig.json ./server/
COPY web/package.json web/tsconfig.json web/tsconfig.app.json web/tsconfig.node.json web/vite.config.ts web/index.html ./web/
RUN pnpm install --frozen-lockfile
COPY protocol ./protocol
COPY server ./server
COPY web ./web
RUN pnpm build

FROM node:22-bookworm-slim AS server
ENV NODE_ENV=production
WORKDIR /app
COPY --from=build /app /app
EXPOSE 8787
CMD ["node", "server/dist/index.js"]

FROM caddy:2.10-alpine AS web
COPY deploy/Caddyfile /etc/caddy/Caddyfile
COPY --from=build /app/web/dist /srv
