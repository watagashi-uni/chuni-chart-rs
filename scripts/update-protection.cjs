#!/usr/bin/env node
// Regenerate the existing random cases with the AGPL chuni-judgement reference.
// Usage: node scripts/update-protection.cjs /path/to/chuni-judgement
const fs = require('node:fs');
const path = require('node:path');
const reference = process.argv[2];
if (!reference) throw Error('Expected chuni-judgement repository path');
const {protect, bands} = require(path.resolve(reference, 'src/judgement.js'));
const file = path.resolve(__dirname, '../tests/fixtures/protection.json');
const cases = JSON.parse(fs.readFileSync(file, 'utf8'));
for (const c of cases) {
  // These synthetic inputs use seconds; their legacy tick fields are all zero.
  const notes = c.notes.map(({tick, ...note}) => note);
  c.expected = protect(notes, c.easy).map(n => ({
    note: {type: n.type, time: n.time, lane: n.lane, width: n.width},
    lanes: Array.from({length: n.width}, (_, i) => bands(n, n.lane + i, {easy: c.easy})),
  }));
}
fs.writeFileSync(file, JSON.stringify(cases) + '\n');
console.log(`Updated ${cases.length} reference cases`);
