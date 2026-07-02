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

var callDnsTop = rpc.declare({
	object: 'overwatch',
	method: 'dns_top',
	params: [ 'client', 'from', 'to', 'limit' ],
	expect: { domains: [] }
});

var RANGE_DAYS = {
	today: 1,
	week: 7,
	month: 30,
	all: 0
};

return view.extend({
	range: 'week',
	client: null, // null = all

	load: function() {
		return callClients();
	},

	refresh: function() {
		var days = RANGE_DAYS[this.range];
		var to = Math.floor(Date.now() / 1000);
		var from = days > 0 ? to - days * 86400 : 0;

		return callDnsTop(this.client, from, to, 50).then(function(domains) {
			domains = domains || [];
			var tbl = document.getElementById('ow-dns-table');
			var empty = document.getElementById('ow-dns-empty');
			if (!tbl) return;

			if (!domains.length) {
				dom.content(tbl, '');
				if (empty) empty.style.display = '';
				return;
			}
			if (empty) empty.style.display = 'none';

			var max = domains[0].count || 1;
			var rows = domains.map(function(d, i) {
				var pct = max > 0 ? (d.count / max * 100) : 0;
				return E('tr', { class: 'tr' }, [
					E('td', { class: 'td ow-num' }, String(i + 1)),
					// array-wrapped: forces createTextNode, escaping the untrusted domain string
					E('td', { class: 'td' }, [ d.domain || '' ]),
					E('td', { class: 'td ow-num' }, String(d.count)),
					E('td', { class: 'td' }, [
						E('div', { class: 'ow-bar-cell' }, [
							E('div', { class: 'ow-bar' }, [
								E('div', { class: 'ow-bar-fill', style: 'width:' + pct.toFixed(1) + '%' })
							])
						])
					])
				]);
			});

			dom.content(tbl, [
				E('tr', { class: 'tr table-titles' }, [
					E('th', { class: 'th' }, _('#')),
					E('th', { class: 'th' }, _('Domain')),
					E('th', { class: 'th ow-num' }, _('Queries')),
					E('th', { class: 'th' }, _('Share'))
				])
			].concat(rows));
		}).catch(function(err) {
			ui.addNotification(null, E('p', _('Failed to fetch DNS stats: %s').format(err.message)));
		});
	},

	render: function(loadResult) {
		var clients = loadResult || [];
		var self = this;

		var rangeBtns = [
			{ v: 'today', label: _('Today') },
			{ v: 'week', label: _('7 Days') },
			{ v: 'month', label: _('30 Days') },
			{ v: 'all', label: _('All') }
		].map(function(r) {
			return E('button', {
				type: 'button',
				class: 'ow-seg-btn' + (r.v === self.range ? ' active' : ''),
				'data-range': r.v,
				click: function() {
					self.range = r.v;
					var btns = document.querySelectorAll('#ow-range-seg .ow-seg-btn');
					for (var i = 0; i < btns.length; i++)
						btns[i].classList.toggle('active', btns[i].getAttribute('data-range') === r.v);
					self.refresh();
				}
			}, r.label);
		});

		var clientOptions = [ E('option', { value: '' }, _('All clients')) ].concat(
			clients.map(function(c) {
				// array-wrapped: forces createTextNode, escaping the untrusted hostname
				return E('option', { value: c.mac }, [ (c.host || c.mac) + ' (' + c.ip + ')' ]);
			})
		);
		var clientSelect = E('select', { id: 'ow-dns-client-select' }, clientOptions);
		clientSelect.addEventListener('change', L.bind(function(ev) {
			this.client = ev.target.value || null;
			this.refresh();
		}, this));

		var node = E([], [
			E('link', { rel: 'stylesheet', href: L.resource('view/overwatch/overwatch.css') }),

			E('h2', {}, _('Overwatch — DNS Queries')),

			E('div', { class: 'ow-toolbar' }, [
				E('div', { class: 'ow-seg', id: 'ow-range-seg' }, rangeBtns),
				clientSelect,
				E('span', { class: 'ow-toolbar-spacer' }),
				E('button', {
					type: 'button',
					class: 'cbi-button cbi-button-action',
					click: function() { self.refresh(); }
				}, _('Refresh'))
			]),

			E('div', { id: 'ow-dns-empty', class: 'ow-chart-empty', style: 'display:none' },
				_('No DNS query data yet. Enable "dns_stats_enabled" in the Overwatch config to start collecting (opt-in).')),

			E('table', { class: 'table', id: 'ow-dns-table' })
		]);

		this.refresh();

		return node;
	}
});
