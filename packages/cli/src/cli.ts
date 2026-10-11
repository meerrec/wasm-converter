#!/usr/bin/env node
/** Точка входа bin `doc-converter`. */

import { run } from './run.js';

process.exitCode = await run(process.argv.slice(2));
