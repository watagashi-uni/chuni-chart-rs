#!/usr/bin/env node
// Compare parsing with the legacy JS port; report (do not reject) expected
// judgement differences after migration to the recovered EXE rules.
// Usage: node scripts/compare-reference.cjs /path/to/chuni-judgement /path/to/chart.c2s ...
const fs = require('node:fs'), path = require('node:path'), cp = require('node:child_process');
const [reference, ...files] = process.argv.slice(2);
if (!reference || !files.length) throw Error('Expected reference repository directory and C2S files');
const {parseC2S} = require(path.resolve(reference, 'src/parser.js'));
const {protect, bands} = require(path.resolve(reference, 'src/judgement.js'));
const binary = process.env.CHUNI_BINARY || path.resolve(__dirname, '../target/release/chuni-chart-rs');
let maxTimingError = 0, totalNotes = 0, totalGround = 0, changedLanes = 0;
function near(a, b, label) {
  const error = Math.abs(a - b); maxTimingError = Math.max(maxTimingError, error);
  if (error > 1e-11) throw Error(`${label}: ${a} != ${b}`);
}
for (const file of files) {
  const js = parseC2S(fs.readFileSync(file, 'utf8'));
  const rs = JSON.parse(cp.execFileSync(binary, ['inspect', file], {maxBuffer:32*1024*1024}));
  if (js.notes.length !== rs.chart.notes.length) throw Error(`Note count: ${file}`);
  for (let i = 0; i < js.notes.length; i++) {
    const a = js.notes[i], b = rs.chart.notes[i];
    if (a.type !== b.type) throw Error(`Source note type: ${file} ${i}`);
    for (const field of ['tick','time','lane','width']) near(a[field], b[field], `${file} ${i} ${field}`);
  }
  const ws = protect(js.notes);
  if (ws.length !== rs.windows.length) throw Error('Ground count');
  for (let i = 0; i < ws.length; i++) {
    const w = ws[i];
    for (let lane = w.lane; lane < w.lane + w.width; lane++) {
      const a = bands(w, lane), b = rs.windows[i].lanes[lane - w.lane];
      if (a.length !== b.length || a.some((x,j) => x.grade !== b[j].grade
          || Math.abs(x.from-b[j].from)>1e-11 || Math.abs(x.to-b[j].to)>1e-11)) changedLanes++;
    }
  }
  totalNotes += js.notes.length; totalGround += ws.length;
}
console.log(JSON.stringify({charts:files.length, notes:totalNotes, ground:totalGround,
  max_parser_timing_error_seconds:maxTimingError, changed_judgement_lanes:changedLanes}));
