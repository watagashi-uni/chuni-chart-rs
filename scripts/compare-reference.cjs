#!/usr/bin/env node
// Optional developer check against the existing AGPL JavaScript implementation.
// Usage: node scripts/compare-reference.cjs /path/to/chuni-judgement /path/to/chart.c2s ...
const fs=require('node:fs'),path=require('node:path'),cp=require('node:child_process');
const [reference,...files]=process.argv.slice(2);
if(!reference||!files.length)throw Error('Expected reference repository directory and C2S files');
const {parseC2S}=require(path.resolve(reference,'src/parser.js'));
const {protect,bands}=require(path.resolve(reference,'src/judgement.js'));
const binary=process.env.CHUNI_BINARY||path.resolve(__dirname,'../target/release/chuni-chart-rs');
let maxError=0,totalNotes=0,totalGround=0;
function near(a,b,label){const error=Math.abs(a-b);maxError=Math.max(maxError,error);if(error>1e-11)throw Error(`${label}: ${a} != ${b}`);}
for(const file of files){const js=parseC2S(fs.readFileSync(file,'utf8'));const rs=JSON.parse(cp.execFileSync(binary,['inspect',file],{maxBuffer:32*1024*1024}));
 if(js.notes.length!==rs.chart.notes.length)throw Error(`Note count: ${file}`);
 for(let i=0;i<js.notes.length;i++){const a=js.notes[i],b=rs.chart.notes[i];if(a.type!==b.type)throw Error(`Note type: ${file} ${i}`);for(const field of ['tick','time','lane','width'])near(a[field],b[field],`${file} ${i} ${field}`);}
 const ws=protect(js.notes);if(ws.length!==rs.windows.length)throw Error('Ground count');
 for(let i=0;i<ws.length;i++){const w=ws[i];for(let l=w.lane;l<w.lane+w.width;l++){const a=bands(w,l),b=rs.windows[i].lanes[l-w.lane];if(a.length!==b.length)throw Error(`Band count ${file} ${i} ${l}`);for(let j=0;j<a.length;j++){if(a[j].grade!==b[j].grade)throw Error('Grade mismatch');near(a[j].from,b[j].from,'early');near(a[j].to,b[j].to,'late');}}}
 totalNotes+=js.notes.length;totalGround+=ws.length;
}
console.log(JSON.stringify({charts:files.length,notes:totalNotes,ground:totalGround,max_error_seconds:maxError}));
