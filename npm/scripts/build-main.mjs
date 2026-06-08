// Generates the publishable main package (@coregraph/cli) into npm/dist/cli/.
// The version is stamped from the cli crate and the per-platform optional
// dependencies are pinned to that exact version.
import { mkdirSync, writeFileSync, copyFileSync, rmSync } from 'node:fs';
import { join } from 'node:path';
import {
  REPO_ROOT,
  MAIN_PACKAGE,
  BIN,
  LICENSE,
  REPOSITORY,
  HOMEPAGE,
  PLATFORMS,
  platformPackageName,
  cliVersion,
} from '../config.mjs';

const version = cliVersion();
const outDir = join(REPO_ROOT, 'npm', 'dist', 'cli');
rmSync(outDir, { recursive: true, force: true });
mkdirSync(join(outDir, 'bin'), { recursive: true });

// Exact-pin each platform package so the main package and its native binary
// can never resolve to mismatched versions.
const optionalDependencies = {};
for (const p of PLATFORMS) {
  optionalDependencies[platformPackageName(p)] = version;
}

const pkg = {
  name: MAIN_PACKAGE,
  version,
  description: 'CoreGraph — a queryable code symbol graph CLI (tree-sitter + stack-graphs).',
  keywords: [
    'code-graph',
    'symbol-graph',
    'tree-sitter',
    'stack-graphs',
    'static-analysis',
    'cli',
    'mcp',
    'lsp',
  ],
  homepage: HOMEPAGE,
  repository: { type: 'git', url: `git+${REPOSITORY}.git` },
  license: LICENSE,
  bin: { [BIN]: 'bin/coregraph.js' },
  files: ['bin/'],
  engines: { node: '>=18' },
  optionalDependencies,
  publishConfig: { access: 'public' },
};

writeFileSync(join(outDir, 'package.json'), `${JSON.stringify(pkg, null, 2)}\n`);
copyFileSync(
  join(REPO_ROOT, 'npm', 'launcher', 'coregraph.js'),
  join(outDir, 'bin', 'coregraph.js')
);
copyFileSync(join(REPO_ROOT, 'npm', 'README.npm.md'), join(outDir, 'README.md'));
// Ship the license text in the package — npm always packs a LICENSE file, so
// the published MIT package carries its license, not just the SPDX field.
copyFileSync(join(REPO_ROOT, 'LICENSE'), join(outDir, 'LICENSE'));

console.log(`built main package ${MAIN_PACKAGE}@${version} -> ${outDir}`);
