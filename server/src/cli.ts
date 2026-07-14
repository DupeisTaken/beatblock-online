#!/usr/bin/env node
import { Command } from 'commander';
import type { Role } from '@bbt/protocol';
import { AuthService } from './auth-service.js';
import { loadConfig } from './config.js';
import { createStore } from './store-factory.js';

const config = loadConfig();
const store = createStore(config);
await store.migrate();
const auth = new AuthService(store, config);
const program = new Command()
  .name('bbtctl')
  .description('Beatblock Together instance administration')
  .version('0.1.0-alpha.1');

program
  .command('invite-create')
  .requiredOption('--role <role>', 'player or organizer')
  .option('--uses <count>', 'redemption count', '1')
  .option('--expires-hours <hours>', 'hours until expiration', '168')
  .action(async (args) => {
    if (!['player', 'organizer'].includes(args.role))
      throw new Error('role must be player or organizer');
    const result = await auth.createInvite(
      args.role as Exclude<Role, 'operator'>,
      Number(args.uses),
      Date.now() + Number(args.expiresHours) * 3_600_000,
    );
    console.log(
      JSON.stringify(
        {
          inviteId: result.invite.id,
          code: result.code,
          role: result.invite.role,
          expiresAtMs: result.invite.expiresAtMs,
        },
        null,
        2,
      ),
    );
  });
program
  .command('invite-list')
  .action(async () => console.log(JSON.stringify(await store.listInvites(), null, 2)));
program
  .command('invite-revoke')
  .argument('<id>')
  .action(async (inviteId) => {
    const invite = await store.getInvite(inviteId);
    if (!invite) throw new Error('Invite not found');
    invite.revokedAtMs = Date.now();
    await store.updateInvite(invite);
  });
program
  .command('user-list')
  .action(async () => console.log(JSON.stringify(await store.listUsers(), null, 2)));
program
  .command('user-role')
  .argument('<id>')
  .argument('<role>')
  .action(async (userId, role) => {
    if (!['player', 'organizer', 'operator'].includes(role)) throw new Error('Invalid role');
    const user = await store.getUser(userId);
    if (!user) throw new Error('User not found');
    user.role = role as Role;
    await store.updateUser(user);
  });
program
  .command('user-disable')
  .argument('<id>')
  .action(async (userId) => {
    const user = await store.getUser(userId);
    if (!user) throw new Error('User not found');
    user.disabled = true;
    await store.updateUser(user);
  });
program
  .command('user-enable')
  .argument('<id>')
  .action(async (userId) => {
    const user = await store.getUser(userId);
    if (!user) throw new Error('User not found');
    user.disabled = false;
    await store.updateUser(user);
  });
program
  .command('session-reset')
  .argument('<userId>')
  .action(async (userId) => {
    for (const session of await store.listSessions(userId)) {
      session.revokedAtMs = Date.now();
      await store.updateSession(session);
    }
  });
program
  .command('allow-mod')
  .argument('<id>')
  .argument('<hash>')
  .action(async (id, hash) => store.addAllowedMod(id, hash));
program
  .command('remove-mod')
  .argument('<id>')
  .action(async (id) => store.removeAllowedMod(id));
program
  .command('list-mods')
  .action(async () => console.log(JSON.stringify(await store.listAllowedMods(), null, 2)));
program
  .command('retention-prune')
  .option('--days <days>', 'days to retain', String(config.runEventRetentionDays))
  .action(async (args) =>
    console.log(
      JSON.stringify({
        deleted: await store.pruneRunEvents(Date.now() - Number(args.days) * 86_400_000),
      }),
    ),
  );
program
  .command('status')
  .action(async () => console.log(JSON.stringify(await store.getStatus(), null, 2)));

try {
  await program.parseAsync(process.argv);
} finally {
  await store.close();
}
