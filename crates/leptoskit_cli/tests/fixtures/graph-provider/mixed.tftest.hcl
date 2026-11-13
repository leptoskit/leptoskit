// Every provider is mocked; this exercises expressions, never cloud resources.
mock_provider "cloudflare" {
}
mock_provider "digitalocean" {
  mock_resource "digitalocean_droplet" { defaults = { id = "201" } }
}
mock_provider "hcloud" {
  mock_resource "hcloud_firewall" { defaults = { id = "101" } }
}
variables { origin_csrs = { web = "public-csr-fixture", compiler = "public-csr-fixture" } }
override_resource {
  target = digitalocean_droplet.host["compiler"]
  values = { id = "201", ipv4_address = "192.0.2.20", ipv6_address = "2001:db8::20" }
}
override_resource {
  target = hcloud_server.host["web"]
  values = { id = "201", ipv4_address = "192.0.2.10", ipv6_address = "2001:db8::10" }
}
run "expanded_boundaries" {
  command = apply
  assert {
    condition     = toset([for rule in digitalocean_firewall.host["compiler"].inbound_rule : rule.port_range]) == toset(["22", "443"])
    error_message = "Expanded provider boundary differs from the fixture contract."
  }
  assert {
    condition     = alltrue([for rule in digitalocean_firewall.host["compiler"].inbound_rule : toset(rule.source_addresses) == (rule.port_range == "22" ? toset(["192.0.2.0/24", "2001:db8::/32"]) : toset(["173.245.48.0/20", "2400:cb00::/32"]))])
    error_message = "Expanded provider boundary differs from the fixture contract."
  }
  assert {
    condition     = cloudflare_dns_record.ipv4["play-api.leptoskit.dev"].content == "192.0.2.20"
    error_message = "Expanded provider boundary differs from the fixture contract."
  }
  assert {
    condition     = cloudflare_dns_record.ipv6["play-api.leptoskit.dev"].content == "2001:db8::20"
    error_message = "Expanded provider boundary differs from the fixture contract."
  }
  assert {
    condition     = toset([for rule in hcloud_firewall.host["web"].rule : rule.port]) == toset(["22", "443"])
    error_message = "Expanded provider boundary differs from the fixture contract."
  }
  assert {
    condition     = alltrue([for rule in hcloud_firewall.host["web"].rule : toset(rule.source_ips) == (rule.port == "22" ? toset(["192.0.2.0/24", "2001:db8::/32"]) : toset(["173.245.48.0/20", "2400:cb00::/32"]))])
    error_message = "Expanded provider boundary differs from the fixture contract."
  }
  assert {
    condition     = cloudflare_dns_record.ipv4["leptoskit.dev"].content == "192.0.2.10"
    error_message = "Expanded provider boundary differs from the fixture contract."
  }
  assert {
    condition     = cloudflare_dns_record.ipv6["leptoskit.dev"].content == "2001:db8::10"
    error_message = "Expanded provider boundary differs from the fixture contract."
  }
}
