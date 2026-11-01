// Splits a text bundle of `=== relative/path` sections into files under a target directory.
// Usage: node scripts/unbundle.mjs <bundle> <targetDir>
import { mkdirSync, readFileSync, writeFileSync } from 'node:fs';
import { dirname, join } from 'node:path';

const [bundle, target] = process.argv.slice(2);
if (!bundle || !target) {
  console.error('usage: node scripts/unbundle.mjs <bundle> <targetDir>');
  process.exit(1);
}
const sections = readFileSync(bundle, 'utf8').split(/^=== (.+)$/m);
let count = 0;
for (let i = 1; i < sections.length; i += 2) {
  const rel = sections[i].trim();
  const body = sections[i + 1].replace(/^\r?\n/, '');
  const out = join(target, rel);
  mkdirSync(dirname(out), { recursive: true });
  writeFileSync(out, body);
  count++;
}
console.log(`wrote ${count} files to ${target}`);
