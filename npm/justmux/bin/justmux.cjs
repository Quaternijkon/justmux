#!/usr/bin/env node
'use strict';

try {
  require('../lib/launcher.cjs').run(process.argv.slice(2));
} catch (error) {
  process.stderr.write(`justmux: ${error.message}\n`);
  process.exitCode = 1;
}
