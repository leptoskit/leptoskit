// Every provider is mocked; this exercises expressions, never cloud resources.
mock_provider "cloudflare" {
}
mock_provider "hcloud" {
  mock_resource "hcloud_firewall" { defaults = { id = "101" } }
}
variables { origin_csrs = { web = "public-csr-fixture", compiler = "public-csr-fixture" } }
override_resource {
  target = hcloud_server.host["web"]
  values = { id = "201", ipv4_address = "192.0.2.10", ipv6_address = "2001:db8::10" }
}
run "expanded_boundaries" {
  command = apply
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
