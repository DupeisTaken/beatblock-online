import type { Config } from './config.js';
import type { Store } from './models.js';
import { MemoryStore } from './memory-store.js';
import { PostgresStore } from './postgres-store.js';

export function createStore(config: Config): Store {
  return config.databaseUrl === 'memory://'
    ? new MemoryStore()
    : new PostgresStore(config.databaseUrl);
}
