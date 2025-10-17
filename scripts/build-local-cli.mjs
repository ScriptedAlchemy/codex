#!/usr/bin/env node
/**
 * Build the Codex CLI for local use and stage it under dist/ so that
 * `node dist/bin/codex.js …` works (or you can `npm link` from dist).
 *
 * This is a host-only build: it compiles the native CLI for your machine
 * and places it at dist/vendor/<target>/codex/(codex|codex.exe) so the
 * existing Node wrapper resolves it correctly.
 */
import { spawn } from 'node:child_process';
import { cpSync, existsSync, mkdirSync, chmodSync } from 'node:fs';
import path from 'node:path';
import { fileURLToPath } from 'node:url';

const __filename = fileURLToPath(import.meta.url);
const __dirname = path.dirname(__filename);
const REPO_ROOT = path.resolve(__dirname, '..');

function sh(cmd, args, opts = {}) {
  return new Promise((resolve, reject) => {
    const child = spawn(cmd, args, {
      stdio: 'inherit',
      ...opts,
    });
    child.on('error', reject);
    child.on('exit', (code) => {
      if (code === 0) resolve();
      else reject(new Error(`${cmd} exited with code ${code}`));
    });
  });
}

function hostTargetTriple() {
  const { platform, arch } = process;
  if (platform === 'darwin') {
    if (arch === 'arm64') return 'aarch64-apple-darwin';
    if (arch === 'x64') return 'x86_64-apple-darwin';
  } else if (platform === 'linux') {
    if (arch === 'arm64') return 'aarch64-unknown-linux-musl';
    if (arch === 'x64') return 'x86_64-unknown-linux-musl';
  } else if (platform === 'win32') {
    if (arch === 'arm64') return 'aarch64-pc-windows-msvc';
    if (arch === 'x64') return 'x86_64-pc-windows-msvc';
  }
  throw new Error(`Unsupported platform ${platform}/${arch}`);
}

async function main() {
  const distDir = path.join(REPO_ROOT, 'dist');
  const codexJsSrc = path.join(REPO_ROOT, 'codex-cli', 'bin', 'codex.js');
  const codexJsDstDir = path.join(distDir, 'bin');
  const codexRsDir = path.join(REPO_ROOT, 'codex-rs');

  // 1) Build native CLI for host
  console.log('• Building native CLI (release)…');
  await sh('cargo', ['build', '-p', 'codex-cli', '--release'], { cwd: codexRsDir });

  // 2) Stage Node wrapper under dist/bin
  console.log('• Staging Node wrapper…');
  mkdirSync(codexJsDstDir, { recursive: true });
  const codexJsDst = path.join(codexJsDstDir, 'codex.js');
  cpSync(codexJsSrc, codexJsDst);

  // 3) Create vendor layout expected by the wrapper and copy the binary
  const triple = hostTargetTriple();
  const vendorCodexDir = path.join(distDir, 'vendor', triple, 'codex');
  mkdirSync(vendorCodexDir, { recursive: true });

  const exe = process.platform === 'win32' ? 'codex.exe' : 'codex';
  const builtPath = path.join(codexRsDir, 'target', 'release', exe);
  const stagedPath = path.join(vendorCodexDir, exe);

  if (!existsSync(builtPath)) {
    throw new Error(`Built binary not found: ${builtPath}`);
  }
  cpSync(builtPath, stagedPath);
  if (process.platform !== 'win32') {
    try { chmodSync(stagedPath, 0o755); } catch {}
  }

  console.log('\n✓ Local CLI staged at:');
  console.log(`  ${path.relative(REPO_ROOT, path.join('dist', 'bin', 'codex.js'))}`);
  console.log('\nTry:');
  console.log('  node dist/bin/codex.js --version');
  console.log('  node dist/bin/codex.js --help');
  console.log('\nOr link globally from dist/:');
  console.log('  (cd dist && npm link)');
}

main().catch((err) => {
  console.error(err);
  process.exit(1);
});

