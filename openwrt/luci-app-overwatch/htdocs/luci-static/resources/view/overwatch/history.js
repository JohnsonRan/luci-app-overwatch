'use strict';
'require view';
'require rpc';
'require ui';
'require dom';

var callClients = rpc.declare({
	object: 'overwatch',
	method: 'clients',
	expect: { clients: [] }
});

var callHistory = rpc.declare({
	object: 'overwatch',
	method: 'history',
	params: [ 'tier', 'client', 'from', 'to', 'max_points' ],
	expect: { points: [] }
});

var callSummary = rpc.declare({
	object: 'overwatch',
	method: 'summary',
	params: [ 'top_n' ],
	expect: {}
});

var TIER_X_UNIT = {
	realtime: 'minute',
	day: 'hour',
	week: 'day',
	month: 'day'
};

function fmtBits(bits) {
	var bytes = bits / 8;
	if (bytes >= 1e9) return (bytes / 1e9).toFixed(2) + ' GB/s';
	if (bytes >= 1e6) return (bytes / 1e6).toFixed(2) + ' MB/s';
	if (bytes >= 1e3) return (bytes / 1e3).toFixed(1) + ' KB/s';
	return bytes.toFixed(0) + ' B/s';
}

function fmtBytes(bytes) {
	if (bytes >= 1e12) return (bytes / 1e12).toFixed(2) + ' TB';
	if (bytes >= 1e9) return (bytes / 1e9).toFixed(2) + ' GB';
	if (bytes >= 1e6) return (bytes / 1e6).toFixed(2) + ' MB';
	if (bytes >= 1e3) return (bytes / 1e3).toFixed(1) + ' kB';
	return bytes.toFixed(0) + ' B';
}

// Load one at a time: the adapter/zoom UMD wrappers touch window.Chart at parse
// time, so they must not finish loading before chart.js does.
function loadScriptsSequentially(paths, onDone) {
	if (!paths.length) { onDone(); return; }
	var s = document.createElement('script');
	s.type = 'text/javascript';
	s.src = paths[0];
	s.onload = function() { loadScriptsSequentially(paths.slice(1), onDone); };
	document.head.appendChild(s);
}

return view.extend({
	Chart: null,
	chart: null,
	tier: 'day',
	client: null, // null = all
	clientNames: {}, // mac -> hostname (from the clients call, for the summary table)

	load: function() {
		return callClients();
	},

	setupChart: function() {
		this.Chart = window.Chart;
		this.Chart.register(window.ChartZoom || window['chartjs-plugin-zoom']);
		var ctx = document.getElementById('ow-history-chart').getContext('2d');
		this.chart = new this.Chart(ctx, {
			type: 'line',
			data: { datasets: [
				{ label: _('Download'), data: [], borderColor: '#3b82f6', backgroundColor: 'rgba(59,130,246,0.10)', fill: true, borderWidth: 1.5, pointRadius: 0, tension: 0.15 },
				{ label: _('Upload'), data: [], borderColor: '#ef4444', backgroundColor: 'rgba(239,68,68,0.10)', fill: true, borderWidth: 1.5, pointRadius: 0, tension: 0.15 }
			] },
			options: {
				animation: false,
				responsive: true,
				maintainAspectRatio: false,
				parsing: false,
				interaction: { mode: 'index', intersect: false },
				scales: {
					x: {
						type: 'time',
						time: { unit: TIER_X_UNIT[this.tier] || 'hour' },
						ticks: { maxTicksLimit: 8 },
						grid: { display: false }
					},
					y: {
						beginAtZero: true,
						ticks: { maxTicksLimit: 6, callback: function(v) { return fmtBits(v); } },
						grid: { color: 'rgba(128,128,128,0.15)' }
					}
				},
				plugins: {
					legend: { position: 'top', align: 'end', labels: { boxWidth: 12 } },
					tooltip: {
						callbacks: {
							label: function(c) { return c.dataset.label + ': ' + fmtBits(c.parsed.y); }
						}
					},
					zoom: {
						zoom: { wheel: { enabled: true }, pinch: { enabled: true }, mode: 'x' },
						pan: { enabled: true, mode: 'x' }
					}
				}
			}
		});
		this.refresh();
	},

	setTier: function(tier) {
		this.tier = tier;
		var btns = document.querySelectorAll('#ow-tier-seg .ow-seg-btn');
		for (var i = 0; i < btns.length; i++)
			btns[i].classList.toggle('active', btns[i].getAttribute('data-tier') === tier);
		if (this.chart && this.chart.resetZoom)
			this.chart.resetZoom();
		this.refresh();
	},

	refresh: function() {
		if (!this.chart)
			return Promise.resolve();

		var params = { tier: this.tier, max_points: 500 };
		if (this.client) params.client = this.client;

		return callHistory(params.tier, params.client, params.from, params.to, params.max_points)
			.then(L.bind(function(points) {
				points = points || [];
				this.chart.options.scales.x.time.unit = TIER_X_UNIT[this.tier] || 'hour';
				this.chart.data.datasets[0].data = points.map(function(p) { return { x: p.ts * 1000, y: p.rx_peak_bits }; });
				this.chart.data.datasets[1].data = points.map(function(p) { return { x: p.ts * 1000, y: p.tx_peak_bits }; });
				this.chart.update();
				this.updateStats(points);
			}, this))
			.then(L.bind(function() { return this.refreshSummary(); }, this))
			.catch(function(err) {
				ui.addNotification(null, E('p', _('Failed to fetch history: %s').format(err.message)));
			});
	},

	updateStats: function(points) {
		var empty = document.getElementById('ow-chart-empty');
		if (empty) empty.style.display = points.length ? 'none' : '';

		var el = document.getElementById('ow-hist-stats');
		if (!el) return;

		if (!points.length) {
			dom.content(el, '');
			return;
		}

		var peakRx = 0, peakTx = 0, sumRx = 0, sumTx = 0;
		points.forEach(function(p) {
			var rx = p.rx_peak_bits || 0, tx = p.tx_peak_bits || 0;
			if (rx > peakRx) peakRx = rx;
			if (tx > peakTx) peakTx = tx;
			sumRx += rx;
			sumTx += tx;
		});

		function stat(label, value, cls) {
			return E('span', { class: 'ow-stat' }, [
				E('span', { class: 'ow-stat-label' }, label),
				E('span', { class: 'ow-stat-value' + (cls ? ' ' + cls : '') }, value)
			]);
		}

		dom.content(el, [
			stat(_('Peak Down'), fmtBits(peakRx), 'ow-down'),
			stat(_('Avg Down'), fmtBits(sumRx / points.length), 'ow-down'),
			stat(_('Peak Up'), fmtBits(peakTx), 'ow-up'),
			stat(_('Avg Up'), fmtBits(sumTx / points.length), 'ow-up')
		]);
	},

	refreshSummary: function() {
		return callSummary(10).then(L.bind(function(res) {
			var el = document.getElementById('ow-summary-panel');
			if (!el || !res) return;

			var totalRx = res.total_rx_bytes || 0, totalTx = res.total_tx_bytes || 0;
			var grand = totalRx + totalTx;

			var rows = (res.top || []).map(L.bind(function(t) {
				// array-wrapped: forces createTextNode, escaping the untrusted hostname
				var name = this.clientNames[t.mac];
				var clientCell = name
					? [ E('div', {}, [ name ]), E('div', { class: 'ow-dim' }, [ t.mac ]) ]
					: [ E('div', {}, [ t.mac ]) ];

				var share = grand > 0 ? ((t.rx_total || 0) + (t.tx_total || 0)) / grand * 100 : 0;

				return E('tr', { class: 'tr' }, [
					E('td', { class: 'td' }, clientCell),
					E('td', { class: 'td ow-num' }, fmtBytes(t.rx_total)),
					E('td', { class: 'td ow-num' }, fmtBytes(t.tx_total)),
					E('td', { class: 'td' }, [
						E('div', { class: 'ow-bar-cell' }, [
							E('div', { class: 'ow-bar' }, [
								E('div', { class: 'ow-bar-fill', style: 'width:' + Math.min(share, 100).toFixed(1) + '%' })
							]),
							E('span', { class: 'ow-bar-pct' }, share.toFixed(0) + '%')
						])
					])
				]);
			}, this));

			function card(label, value, cls) {
				return E('div', { class: 'ow-card' }, [
					E('div', { class: 'ow-card-label' }, label),
					E('div', { class: 'ow-card-value' + (cls ? ' ' + cls : '') }, value)
				]);
			}

			dom.content(el, [
				E('div', { class: 'ow-cards' }, [
					card(_('Total Download'), fmtBytes(totalRx), 'ow-down'),
					card(_('Total Upload'), fmtBytes(totalTx), 'ow-up')
				]),
				E('h3', {}, _('Top Clients')),
				E('table', { class: 'table' }, [
					E('tr', { class: 'tr table-titles' }, [
						E('th', { class: 'th' }, _('Client')),
						E('th', { class: 'th ow-num' }, _('Down')),
						E('th', { class: 'th ow-num' }, _('Up')),
						E('th', { class: 'th ow-share-th' }, _('Share'))
					])
				].concat(rows))
			]);
		}, this));
	},

	render: function(loadResult) {
		var clients = loadResult || [];
		var self = this;

		this.clientNames = {};
		clients.forEach(function(c) { self.clientNames[c.mac] = c.host || ''; });

		var tierBtns = [
			{ v: 'realtime', label: _('Realtime') },
			{ v: 'day', label: _('Day') },
			{ v: 'week', label: _('Week') },
			{ v: 'month', label: _('Month') }
		].map(function(t) {
			return E('button', {
				type: 'button',
				class: 'ow-seg-btn' + (t.v === self.tier ? ' active' : ''),
				'data-tier': t.v,
				click: function() { self.setTier(t.v); }
			}, t.label);
		});

		var clientOptions = [ E('option', { value: '' }, _('All clients')) ].concat(
			clients.map(function(c) {
				// array-wrapped: forces createTextNode, escaping the untrusted hostname
				return E('option', { value: c.mac }, [ (c.host || c.mac) + ' (' + c.ip + ')' ]);
			})
		);
		var clientSelect = E('select', { id: 'ow-client-select' }, clientOptions);

		clientSelect.addEventListener('change', L.bind(function(ev) {
			this.client = ev.target.value || null;
			this.refresh();
		}, this));

		loadScriptsSequentially([
			L.resource('view/overwatch/lib/chart.umd.min.js'),
			L.resource('view/overwatch/lib/chartjs-adapter-date-fns.umd.min.js'),
			L.resource('view/overwatch/lib/chartjs-plugin-zoom.umd.min.js')
		], L.bind(this.setupChart, this));

		var node = E([], [
			E('link', { rel: 'stylesheet', href: L.resource('view/overwatch/overwatch.css') }),

			E('h2', {}, _('Overwatch — History')),

			E('div', { class: 'ow-toolbar' }, [
				E('div', { class: 'ow-seg', id: 'ow-tier-seg' }, tierBtns),
				clientSelect,
				E('span', { class: 'ow-toolbar-spacer' }),
				E('button', {
					type: 'button',
					class: 'cbi-button',
					click: function() { if (self.chart && self.chart.resetZoom) self.chart.resetZoom(); }
				}, _('Reset Zoom')),
				E('button', {
					type: 'button',
					class: 'cbi-button cbi-button-action',
					click: function() { self.refresh(); }
				}, _('Refresh'))
			]),

			E('div', { class: 'ow-chart-wrap' }, [
				E('canvas', { id: 'ow-history-chart' }),
				E('div', { id: 'ow-chart-empty', class: 'ow-chart-empty', style: 'display:none' }, _('No data for this period'))
			]),

			E('div', { id: 'ow-hist-stats', class: 'ow-stats' }),

			E('div', { id: 'ow-summary-panel' })
		]);

		return node;
	}
});
