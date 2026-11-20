#!/usr/bin/env node
'use strict';

async function main() {
  const args = process.argv.slice(2);
  const result = await require('../lib/update.cjs').maybeUpdate(args);
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
