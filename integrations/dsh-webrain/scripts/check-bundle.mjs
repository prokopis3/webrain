#!/usr/bin/env node
/**
 * Validate the dsh-webrain bundle before it is installed or published.
 *
 * Checks the manifest, the Loader patch, every declared locale, the detail
 * document, and the artwork size limits the DeepSeek Harness plugin manager
 * enforces. Dependency-free on purpose: CI has no Node packages installed.
 *
 *   node integrations/dsh-webrain/scripts/check-bundle.mjs
 */
import { readFileSync, readdirSync, statSync } from 'node:fs';
import { dirname, join, relative, resolve } from 'node:path';
import { fileURLToPath } from 'node:url';

const root = resolve(dirname(fileURLToPath(import.meta.url)), '..');
const failures = [];
const ok = [];

function fail(message) {
  failures.push(message);
}

function note(message) {
  ok.push(message);
}

function readJson(file) {
  try {
    return JSON.parse(readFileSync(join(root, file), 'utf8'));
  } catch (error) {
    fail(`${file}: ${error.message}`);
    return undefined;
  }
}

/** English is the fallback every other language chains to, so it must exist. */
const REQUIRED_LOCALES = ['en'];

/** The registry scope this repository publishes under; a rename must be deliberate. */
const EXPECTED_NAME = '@ixnk/dsh-webrain';

const pkg = readJson('package.json');
if (pkg) {
  if (typeof pkg.name !== 'string' || !/^(?:@[a-z0-9][a-z0-9._~-]*\/)?[a-z0-9][a-z0-9._~-]*$/.test(pkg.name)) {
    fail(`package.json: name "${pkg.name}" is not a name the plugin registry accepts`);
  } else if (pkg.name !== EXPECTED_NAME) {
    fail(`package.json: name "${pkg.name}" must be ${EXPECTED_NAME}`);
  }
  if (typeof pkg.version !== 'string' || !/^\d+\.\d+\.\d+(?:-[0-9A-Za-z.-]+)?$/.test(pkg.version)) {
    fail(`package.json: version "${pkg.version}" is not semver`);
  }
  if (pkg.dsh?.bundle?.patch !== './cordis.patch.yml') {
    fail('package.json: dsh.bundle.patch must be "./cordis.patch.yml"');
  }
  for (const exported of ['./locale/*.json', './icon']) {
    if (!Object.hasOwn(pkg.exports ?? {}, exported)) fail(`package.json: exports is missing ${exported}`);
  }
  for (const entry of ['cordis.patch.yml', 'locale/*.json', 'docs', 'icon.svg', 'README.md']) {
    if (!(pkg.files ?? []).includes(entry)) fail(`package.json: files is missing ${entry}`);
  }
  note(`manifest ${pkg.name}@${pkg.version}`);
}

// The patch must insert exactly one MCP client row, over stdio, for webrain.
const patchPath = 'cordis.patch.yml';
let patch = '';
try {
  patch = readFileSync(join(root, patchPath), 'utf8');
} catch (error) {
  fail(`${patchPath}: ${error.message}`);
}
if (patch) {
  const required = [
    ['id', /^\s*-\s*id:\s*webrain-mcp\s*$/m],
    ['plugin name', /^\s*name:\s*'@deepseek-ai\/dsh-mcp-client'\s*$/m],
    ['serverName', /^\s*serverName:\s*webrain\s*$/m],
    ['stdio transport', /^\s*transport:\s*stdio\s*$/m],
    ['command', /^\s*command:\s*webrain\s*$/m],
    ['args', /^\s*args:\s*\['mcp'\]\s*$/m],
    ['fingerprint noise off', /^\s*WEBRAIN_STEALTH_NOISE:\s*'0'\s*$/m],
    ['visible startup failure', /^\s*failOnStartupError:\s*true\s*$/m],
  ];
  for (const [label, pattern] of required) {
    if (!pattern.test(patch)) fail(`${patchPath}: missing ${label}`);
  }
  const rowIds = [...patch.matchAll(/^\s*-\s*id:\s*(\S+)\s*$/gm)].map((match) => match[1]);
  if (rowIds.length !== 1) fail(`${patchPath}: expected exactly one inserted row, found ${rowIds.length} (${rowIds.join(', ')})`);
  note('patch inserts exactly one stdio mcp-client row for serverName webrain');
}

// Locales: meta.title, meta.description, and a detail document that exists.
const localeDir = join(root, 'locale');
let locales = [];
try {
  locales = readdirSync(localeDir).filter((name) => name.endsWith('.json'));
} catch (error) {
  fail(`locale/: ${error.message}`);
}
for (const language of REQUIRED_LOCALES) {
  if (!locales.includes(`${language}.json`)) fail(`locale/${language}.json is missing`);
}
const referencedDetails = new Set();
for (const file of locales) {
  const data = readJson(join('locale', file));
  if (!data) continue;
  if (!data.meta?.title) fail(`locale/${file}: meta.title is missing`);
  if (!data.meta?.description) fail(`locale/${file}: meta.description is missing`);
  if (data.meta?.detail) {
    const detail = data.meta.detail;
    if (!detail.endsWith('.md') && !detail.endsWith('.markdown')) {
      fail(`locale/${file}: meta.detail must end in .md or .markdown`);
    }
    if (detail.includes('\\') || detail.startsWith('/') || detail.split('/').includes('..')) {
      fail(`locale/${file}: meta.detail must be a package-relative path with forward slashes`);
    }
    try {
      const size = statSync(join(root, detail)).size;
      if (size > 1024 * 1024) fail(`locale/${file}: ${detail} exceeds the 1 MiB detail limit`);
      referencedDetails.add(detail);
    } catch {
      fail(`locale/${file}: meta.detail ${detail} does not exist`);
    }
  }
}
note(`${locales.length} locales, ${referencedDetails.size} detail documents`);

// A detail document is exactly one image line pointing at a package file.
for (const detail of referencedDetails) {
  const text = readFileSync(join(root, detail), 'utf8').trim();
  const lines = text.split(/\r?\n/).filter((line) => line.trim() !== '');
  const image = /^!\[(?<alt>.+)\]\((?<src>[^)]+)\)$/.exec(lines[0] ?? '');
  if (lines.length !== 1 || !image) {
    fail(`${detail}: must contain exactly one image line, "![alt](image)"`);
    continue;
  }
  const source = join(dirname(join(root, detail)), image.groups.src);
  try {
    const size = statSync(source).size;
    if (size > 5 * 1024 * 1024) fail(`${relative(root, source)} exceeds the 5 MiB detail image limit`);
  } catch {
    fail(`${detail}: image ${image.groups.src} does not exist beside the document`);
  }
}
note('detail documents are one image line with an existing image');

// Icon: an image the plugin manager can decode, under its 256 KiB limit.
try {
  const icon = readFileSync(join(root, 'icon.svg'));
  if (icon.length > 256 * 1024) fail('icon.svg exceeds the 256 KiB icon limit');
  const text = icon.toString('utf8');
  if (!/^\s*<svg[\s>]/.test(text) || !/<\/svg>\s*$/.test(text)) fail('icon.svg is not a complete SVG document');
  if (!text.includes('width="36"') || !text.includes('height="36"') || !text.includes('viewBox="0 0 36 36"')) {
    fail('icon.svg must be a 36x36 viewBox icon');
  }
  for (const banned of ['<script', '<foreignObject', '<image', 'onload=', '<style', '<text']) {
    if (text.includes(banned)) fail(`icon.svg must not contain ${banned}`);
  }
  note(`icon.svg ${icon.length} bytes`);
} catch (error) {
  fail(`icon.svg: ${error.message}`);
}

// A feature image is a self-contained 640x300 illustration with no lettering.
const previews = [];
try {
  for (const file of readdirSync(join(root, 'docs'))) if (file.endsWith('.svg')) previews.push(join('docs', file));
} catch (error) {
  fail(`docs/: ${error.message}`);
}
for (const preview of previews) {
  try {
    const text = readFileSync(join(root, preview), 'utf8');
    if (!text.includes('width="640"') || !text.includes('height="300"') || !text.includes('viewBox="0 0 640 300"')) {
      fail(`${preview} must be a 640x300 viewBox illustration`);
    }
    for (const banned of ['<script', '<foreignObject', 'onload=', '<style', '<text']) {
      if (text.includes(banned)) fail(`${preview} must not contain ${banned}`);
    }
    if (text.length > 1024 * 1024) fail(`${preview} exceeds the 1 MiB recommendation`);
  } catch (error) {
    fail(`${preview}: ${error.message}`);
  }
}
note(`${previews.length} preview illustrations are 640x300 and script-free`);

if (failures.length > 0) {
  console.error(`dsh-webrain bundle check FAILED (${failures.length}):`);
  for (const failure of failures) console.error(`  - ${failure}`);
  process.exit(1);
}
for (const line of ok) console.log(`ok: ${line}`);
console.log(`dsh-webrain bundle check passed (${root})`);
