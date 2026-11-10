// The pages of the seeded views, written as a view's brain would: one
// self-contained document that draws nodus.data in the board's colours.

export const PAGES = {
  'ds-build': `<!doctype html>
<html><head><meta charset="utf-8"><style>
  body { padding: 14px 16px; }
  h1 { font-size: 13px; font-weight: 600; margin: 0 0 2px; }
  .sub { color: var(--nodus-text-3); font-size: 11.5px; margin-bottom: 12px; }
  table { width: 100%; border-collapse: collapse; }
  td { padding: 6px 0; border-top: 1px solid var(--nodus-border); font-size: 12px; }
  td.id { font-family: var(--nodus-mono); color: var(--nodus-text-2); width: 44px; }
  td.branch { color: var(--nodus-text-2); }
  td.num { text-align: right; font-variant-numeric: tabular-nums; width: 56px; }
  .bar { height: 6px; border-radius: 3px; background: var(--nodus-accent); }
  .fail { color: var(--nodus-red); font-weight: 600; }
  td.barcell { width: 90px; padding-left: 10px; }
</style></head><body>
<h1>Docs builds</h1>
<div class="sub" id="sub">Waiting for data…</div>
<table id="rows"></table>
<script>
  nodus.on('data', function (d) {
    if (!d) return;
    document.getElementById('sub').textContent = 'Last six builds · updated ' + d.updated;
    var max = Math.max.apply(null, d.builds.map(function (b) { return b.seconds; }));
    document.getElementById('rows').innerHTML = d.builds.map(function (b) {
      return '<tr><td class="id">' + b.id + '</td><td class="branch">' + b.branch + '</td>' +
        '<td class="num">' + (b.ok ? b.pages + ' pp' : '<span class="fail">failed</span>') + '</td>' +
        '<td class="num">' + (b.ok ? b.seconds + 's' : '–') + '</td>' +
        '<td class="barcell">' + (b.ok ? '<div class="bar" style="width:' + Math.round((b.seconds / max) * 100) + '%"></div>' : '') + '</td></tr>';
    }).join('');
  });
</script>
</body></html>
`,
  'ds-search-size': `<!doctype html>
<html><head><meta charset="utf-8"><style>
  body { padding: 14px 16px; }
  h1 { font-size: 13px; font-weight: 600; margin: 0 0 12px; }
  .big { font-size: 30px; font-weight: 600; font-variant-numeric: tabular-nums; }
  .of { color: var(--nodus-text-3); }
  .track { height: 8px; border-radius: 4px; background: var(--nodus-surface); margin: 12px 0 8px; overflow: hidden; }
  .fill { height: 100%; background: var(--nodus-orange); }
  .sub { color: var(--nodus-text-3); font-size: 11.5px; }
</style></head><body>
<h1>Search index</h1>
<div><span class="big" id="kb">–</span> <span class="of" id="of"></span></div>
<div class="track"><div class="fill" id="fill" style="width:0"></div></div>
<div class="sub" id="sub">Waiting for data…</div>
<script>
  nodus.on('data', function (d) {
    if (!d) return;
    document.getElementById('kb').textContent = (d.kb / 1024).toFixed(1) + ' MB';
    document.getElementById('of').textContent = 'of ' + (d.limit / 1024).toFixed(0) + ' MB';
    document.getElementById('fill').style.width = Math.round((d.kb / d.limit) * 100) + '%';
    document.getElementById('sub').textContent = 'Updated ' + d.updated;
  });
</script>
</body></html>
`
}
