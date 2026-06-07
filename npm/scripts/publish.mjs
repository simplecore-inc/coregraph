// Publishes every assembled package under npm/dist/. Platform packages go first
// so the main package's exact-pinned optionalDependencies resolve immediately.
// Pass --dry-run to validate the publish without uploading.
//
// Auth: npm reads the token from the environment (NODE_AUTH_TOKEN with a
// registry line in .npmrc, as the GitHub Actions setup-node step configures, or
// a logged-in `npm whoami`). This script never embeds a token.
import { readdirSync, existsSync } from 'node:fs';
import { join } from 'node:path';
import { spawnSync } from 'node:child_process';
import { REPO_ROOT, PLATFORMS } from '../config.mjs';

const dryRun = process.argv.includes('--dry-run');
const distRoot = join(REPO_ROOT, 'npm', 'dist');

if (!existsSync(distRoot)) {
  console.error('npm/dist not found — run build-platform.mjs / build-main.mjs first');
  process.exit(2);
}

const dirs = readdirSync(distRoot).filter((d) => d === 'cli' || d.startsWith('cli-'));
const platformDirs = dirs.filter((d) => d !== 'cli').sort();
const ordered = [...platformDirs, ...(dirs.includes('cli') ? ['cli'] : [])];

if (ordered.length === 0) {
  console.error('no packages found under npm/dist');
  process.exit(2);
}

// A real publish must ship every supported platform: the main package lists all
// of them as optionalDependencies, so a missing one yields an install that can
// never resolve a binary on that platform. Allow a partial set only for
// --dry-run (CI validates partial matrices that way).
if (!dryRun) {
  const expected = PLATFORMS.map((p) => `cli-${p.os}-${p.cpu}`);
  const missing = expected.filter((d) => !dirs.includes(d));
  if (missing.length > 0) {
    console.error(`refusing to publish: missing platform packages [${missing.join(', ')}].`);
    console.error('A real publish must include all supported platforms. Rebuild the');
    console.error('missing ones, or use --dry-run to validate a partial set.');
    process.exit(1);
  }
  if (!dirs.includes('cli')) {
    console.error('refusing to publish: main package (npm/dist/cli) not assembled.');
    process.exit(1);
  }
}

for (const dir of ordered) {
  const cwd = join(distRoot, dir);
  const args = ['publish', '--access', 'public'];
  if (dryRun) args.push('--dry-run');
  console.log(`\n$ (cd npm/dist/${dir} && npm ${args.join(' ')})`);
  const r = spawnSync('npm', args, { cwd, stdio: 'inherit' });
  if (r.status !== 0) {
    console.error(`publish failed for ${dir}`);
    process.exit(r.status ?? 1);
  }
}

console.log(dryRun ? '\ndry-run complete' : '\npublish complete');
