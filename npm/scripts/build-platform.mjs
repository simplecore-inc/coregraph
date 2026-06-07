// Generates one publishable per-platform package into npm/dist/cli-<os>-<cpu>/.
// Usage:
//   node npm/scripts/build-platform.mjs --os <os> --cpu <cpu> --binary <path>
// where <os>/<cpu> use Node's process.platform / process.arch vocabulary.
import {
  mkdirSync,
  writeFileSync,
  copyFileSync,
  rmSync,
  chmodSync,
  existsSync,
} from 'node:fs';
import { join } from 'node:path';
import {
  REPO_ROOT,
  LICENSE,
  REPOSITORY,
  platformByKey,
  platformPackageName,
  cliVersion,
} from '../config.mjs';

function arg(name) {
  const i = process.argv.indexOf(`--${name}`);
  return i >= 0 ? process.argv[i + 1] : undefined;
}

const os = arg('os');
const cpu = arg('cpu');
const binaryPath = arg('binary');

if (!os || !cpu || !binaryPath) {
  console.error('usage: build-platform.mjs --os <os> --cpu <cpu> --binary <path>');
  process.exit(2);
}

const plat = platformByKey(os, cpu);
if (!plat) {
  console.error(`unknown platform ${os}-${cpu} (not in config.mjs PLATFORMS)`);
  process.exit(2);
}
if (!existsSync(binaryPath)) {
  console.error(`binary not found: ${binaryPath}`);
  process.exit(2);
}

const version = cliVersion();
const name = platformPackageName(plat);
const outDir = join(REPO_ROOT, 'npm', 'dist', `cli-${os}-${cpu}`);
rmSync(outDir, { recursive: true, force: true });
mkdirSync(join(outDir, 'bin'), { recursive: true });

// `os`/`cpu` make npm install this package only on a matching host, which is
// what lets the main package list all platforms as optional dependencies.
const pkg = {
  name,
  version,
  description: `CoreGraph CLI native binary for ${os}-${cpu}.`,
  homepage: `${REPOSITORY}#readme`,
  repository: { type: 'git', url: `git+${REPOSITORY}.git` },
  license: LICENSE,
  os: [os],
  cpu: [cpu],
  files: ['bin/'],
  publishConfig: { access: 'public' },
};
writeFileSync(join(outDir, 'package.json'), `${JSON.stringify(pkg, null, 2)}\n`);

const dest = join(outDir, 'bin', plat.binary);
copyFileSync(binaryPath, dest);
chmodSync(dest, 0o755);

console.log(`built platform package ${name}@${version} -> ${outDir}`);
