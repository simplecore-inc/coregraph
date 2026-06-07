// Assembles every platform package + the main package from a CI staging
// directory laid out as <staging>/bin-<os>-<cpu>/<binary> (one subdir per
// uploaded build artifact). Drives the per-platform assembly from config.mjs so
// the workflow YAML stays free of a hardcoded platform list.
import { existsSync } from 'node:fs';
import { join } from 'node:path';
import { spawnSync } from 'node:child_process';
import { REPO_ROOT, PLATFORMS } from '../config.mjs';

const staging = process.argv[2];
if (!staging) {
  console.error('usage: assemble-from-artifacts.mjs <staging-dir>');
  process.exit(2);
}

function run(args) {
  const r = spawnSync('node', args, { stdio: 'inherit' });
  if (r.status !== 0) process.exit(r.status ?? 1);
}

let assembled = 0;
for (const p of PLATFORMS) {
  const binary = join(staging, `bin-${p.os}-${p.cpu}`, p.binary);
  if (!existsSync(binary)) {
    console.warn(`! skipping ${p.os}-${p.cpu}: binary not found at ${binary}`);
    continue;
  }
  run([
    join(REPO_ROOT, 'npm', 'scripts', 'build-platform.mjs'),
    '--os', p.os,
    '--cpu', p.cpu,
    '--binary', binary,
  ]);
  assembled += 1;
}

if (assembled === 0) {
  console.error('no platform binaries found in staging — nothing to assemble');
  process.exit(1);
}

// The main package always lists all supported platforms in optionalDependencies
// (from config.mjs), independent of how many were assembled this run.
run([join(REPO_ROOT, 'npm', 'scripts', 'build-main.mjs')]);

console.log(`assembled ${assembled} platform package(s) + main package`);
