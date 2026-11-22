#!/usr/bin/env node
'use strict';

async function main() {
  const args = process.argv.slice(2);
  const updater = require('../lib/update.cjs');
  const result = updater.isUpdateCommand(args)
    ? await updater.runUpdate(args.slice(1))
    : await updater.maybeUpdate(args);
  if (result.action === 'exit') {
    process.exitCode = result.code;
    return;
  }
  require('../lib/launcher.cjs').run(args);
}

main().catch((error) => {
  process.stderr.write(`justmux: ${error.message}\n`);
  process.exitCode = 1;
});
