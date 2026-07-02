'use strict';
'require view';
'require rpc';
'require poll';
'require ui';

var callClients = rpc.declare({
	object: 'overwatch',
	method: 'clients',
	expect: { clients: [] }
});

var MAX_TOTAL_SAMPLES = 300; // 300 * 2s = 10 min
var MAX_SPARK_SAMPLES = 30;  // 30 * 2s = 1 min

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

return view.extend({
	totalHistory: [],
	sparkHistory: {},
	sparkCharts: {},
	totalChart: null,
	Chart: null,

	load: function() {
		return callClients();
	},

	setupChart: function() {
		this.Chart = window.Chart;
		var ctx = document.getElementById('ow-total-chart').getContext('2d');
		this.totalChart = new this.Chart(ctx, {
			type: 'line',
			data: {
				labels: [],
				datasets: [
					{ label: _('Download'), data: [], borderColor: '#3b82f6', backgroundColor: 'rgba(59,130,246,0.12)', fill: true, borderWidth: 1.5, tension: 0.2, pointRadius: 0 },
					{ label: _('Upload'), data: [], borderColor: '#ef4444', backgroundColor: 'rgba(239,68,68,0.12)', fill: true, borderWidth: 1.5, tension: 0.2, pointRadius: 0 }
				]
			},
			options: {
				animation: false,
				responsive: true,
				maintainAspectRatio: false,
				interaction: { mode: 'index', intersect: false },
				scales: {
					x: { ticks: { maxTicksLimit: 6 }, grid: { display: false } },
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
					}
				}
			}
		});
	},

	updateCards: function(vals) {
		var map = {
			'ow-card-down': vals.down,
			'ow-card-up': vals.up,
			'ow-card-active': vals.active,
			'ow-card-total': vals.total
		};
		for (var id in map) {
			var el = document.getElementById(id);
			if (el) el.textContent = map[id];
		}
	},

	renderSparkline: function(mac, history) {
		var canvas = document.querySelector('canvas.ow-sparkline[data-mac="' + mac + '"]');
		if (!canvas || !this.Chart) return;
		this.sparkCharts[mac] = new this.Chart(canvas.getContext('2d'), {
			type: 'line',
			data: { labels: history.map(function(v, i) { return i; }), datasets: [
				{ data: history.slice(), borderColor: '#3b82f6', borderWidth: 1, pointRadius: 0, tension: 0.3 }
			] },
			options: {
				animation: false, responsive: false,
				scales: { x: { display: false }, y: { display: false, beginAtZero: true } },
				plugins: { legend: { display: false }, tooltip: { enabled: false } }
			}
		});
	},

	poll: function() {
		return callClients().then(L.bind(function(clients) {
			clients = clients || [];
			var now = Date.now();
			var totalRx = 0, totalTx = 0;

			var activeCount = 0, totalBytes = 0;
			clients.forEach(function(c) {
				totalRx += c.rx_bps || 0;
				totalTx += c.tx_bps || 0;
				if ((c.rx_bps || 0) + (c.tx_bps || 0) > 0) activeCount++;
				totalBytes += (c.rx_total || 0) + (c.tx_total || 0);
				var h = this.sparkHistory[c.mac] || (this.sparkHistory[c.mac] = []);
				h.push(c.rx_bps || 0);
				if (h.length > MAX_SPARK_SAMPLES) h.shift();
			}, this);

			this.updateCards({
				down: fmtBits(totalRx),
				up: fmtBits(totalTx),
				active: activeCount + ' / ' + clients.length,
				total: fmtBytes(totalBytes)
			});

			this.totalHistory.push({ ts: now, rx: totalRx, tx: totalTx });
			if (this.totalHistory.length > MAX_TOTAL_SAMPLES) this.totalHistory.shift();

			this.renderTable(clients);

			if (this.totalChart) {
				this.totalChart.data.labels = this.totalHistory.map(function(s) {
					return new Date(s.ts).toLocaleTimeString();
				});
				this.totalChart.data.datasets[0].data = this.totalHistory.map(function(s) { return s.rx; });
				this.totalChart.data.datasets[1].data = this.totalHistory.map(function(s) { return s.tx; });
				this.totalChart.update('none');
			}

			clients.forEach(function(c) {
				this.renderSparkline(c.mac, this.sparkHistory[c.mac] || []);
			}, this);
		}, this)).catch(function(err) {
			ui.addNotification(null, E('p', _('Failed to fetch client list: %s').format(err.message)));
		});
	},

	renderTable: function(clients) {
		// destroy prior tick's sparkline charts before cbi_update_table discards their canvases (else they leak)
		for (var mac in this.sparkCharts) {
			this.sparkCharts[mac].destroy();
		}
		this.sparkCharts = {};

		var rows = clients.map(function(c) {
			var rx_bps = c.rx_bps || 0, tx_bps = c.tx_bps || 0;
			var rx_total = c.rx_total || 0, tx_total = c.tx_total || 0;
			return [
				// array-wrapped: forces createTextNode, escaping the untrusted hostname
				E('span', {}, [ c.host || c.mac ]),
				c.ip,
				// data-value = raw number so header-click sort is numeric, not by formatted text
				E('span', { class: 'ow-speed', 'data-value': rx_bps }, fmtBits(rx_bps)),
				E('span', { class: 'ow-speed', 'data-value': tx_bps }, fmtBits(tx_bps)),
				E('span', { 'data-value': rx_total }, fmtBytes(rx_total)),
				E('span', { 'data-value': tx_total }, fmtBytes(tx_total)),
				String(c.conns_tcp) + ' / ' + String(c.conns_udp),
				E('canvas', { class: 'ow-sparkline', 'data-mac': c.mac, width: 80, height: 24 })
			];
		});

		cbi_update_table('#ow-client-table', rows, E('em', _('No clients seen yet.')));
	},

	render: function(loadResult) {
		var node = E([], [
			E('link', { rel: 'stylesheet', href: L.resource('view/overwatch/overwatch.css') }),
			E('script', {
				type: 'text/javascript',
				src: L.resource('view/overwatch/lib/chart.umd.min.js'),
				load: L.bind(function() { this.setupChart(); this.poll(); }, this)
			}),

			E('h2', {}, _('Overwatch — Dashboard')),

			E('div', { class: 'ow-cards' }, [
				E('div', { class: 'ow-card' }, [
					E('div', { class: 'ow-card-label' }, _('Download')),
					E('div', { class: 'ow-card-value ow-down', id: 'ow-card-down' }, '—')
				]),
				E('div', { class: 'ow-card' }, [
					E('div', { class: 'ow-card-label' }, _('Upload')),
					E('div', { class: 'ow-card-value ow-up', id: 'ow-card-up' }, '—')
				]),
				E('div', { class: 'ow-card' }, [
					E('div', { class: 'ow-card-label' }, _('Active Clients')),
					E('div', { class: 'ow-card-value', id: 'ow-card-active' }, '—')
				]),
				E('div', { class: 'ow-card' }, [
					E('div', { class: 'ow-card-label' }, _('Total Traffic')),
					E('div', { class: 'ow-card-value', id: 'ow-card-total' }, '—')
				])
			]),

			E('div', { class: 'ow-chart-wrap' }, [
				E('canvas', { id: 'ow-total-chart' })
			]),

			E('table', { class: 'table cbi-section-table ow-table', id: 'ow-client-table' }, [
				E('tr', { class: 'tr table-titles' }, [
					E('th', { class: 'th' }, _('Host')),
					E('th', { class: 'th' }, _('IP')),
					E('th', { class: 'th' }, _('Down')),
					E('th', { class: 'th' }, _('Up')),
					E('th', { class: 'th' }, _('Total Down')),
					E('th', { class: 'th' }, _('Total Up')),
					E('th', { class: 'th' }, _('TCP / UDP')),
					E('th', { class: 'th' }, _('Trend'))
				])
			])
		]);

		poll.add(L.bind(this.poll, this), 2);

		document.addEventListener('visibilitychange', function() {
			if (document.hidden) poll.stop();
			else poll.start();
		});

		return node;
	}
});
