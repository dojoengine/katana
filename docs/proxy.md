# HTTP(S) Proxy Support

Katana honors the standard proxy environment variables for its outbound HTTP connections. This
allows running Katana in environments where egress traffic must go through a forward proxy (e.g.,
corporate networks, sandboxed VMs).

## Environment Variables

| Variable      | Effect                                                          |
| ------------- | --------------------------------------------------------------- |
| `HTTP_PROXY`  | Proxy used for plain `http://` targets                          |
| `HTTPS_PROXY` | Proxy used for `https://` targets (via HTTP `CONNECT` tunneling) |
| `NO_PROXY`    | Comma-separated list of hosts/domains/CIDRs to connect directly |

Lowercase variants (`http_proxy`, `https_proxy`, `no_proxy`) are also recognized. The variables
are read when a client is constructed, i.e., at node startup.

## Covered Connections

All of Katana's outbound HTTP clients honor these variables:

- **Forking** (`--forking.provider`) -- the Starknet JSON-RPC client used to fetch remote state
- **Full-node sync** -- the JSON-RPC sync source
- **L1/settlement providers** -- Ethereum and Starknet settlement-layer RPC clients
- **Messaging** -- the L1 JSON-RPC clients used by the messaging service
- **Paymaster proxy** -- forwarding paymaster API requests to an upstream paymaster service
- **Cartridge API and VRF clients**

Most of these are backed by [`reqwest`], which supports the proxy variables natively. The
Starknet JSON-RPC client (`katana-starknet`) uses a reqwest-backed transport implemented in
`crates/starknet/src/http.rs` for the same behavior.

## Loopback Bypass

Connections to loopback targets (`localhost`, `127.0.0.1`, `::1`) never go through the proxy,
even when the proxy variables are set and `NO_PROXY` does not list them. Local endpoints --
sidecars (paymaster, VRF), a Katana node on the same host, tests -- must stay reachable without
requiring users to also maintain a `NO_PROXY` entry.

## Limitations

- SOCKS proxies (`socks5://` URLs in the proxy variables) are not supported; only HTTP(S)
  forward proxies are.
- OS-level proxy settings (macOS System Settings, Windows registry) are not read; only the
  environment variables are.

[`reqwest`]: https://docs.rs/reqwest
