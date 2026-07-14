#!/usr/bin/env node
import { buildApp } from './app.js';
import { loadConfig } from './config.js';
import { createStore } from './store-factory.js';

const config = loadConfig();
if (
  process.env.NODE_ENV === 'production' &&
  !config.allowInsecureHttp &&
  !config.publicUrl.startsWith('https://')
)
  throw new Error('PUBLIC_URL must use HTTPS in production');
const store = createStore(config);
await store.migrate();
const app = await buildApp({ config, store });
await app.listen({ host: config.host, port: config.port });
