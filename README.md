# mcproxy

Reverse TCP proxy for a Minecraft server

## Features

- keeps the origin IP out of DNS
- per-IP connection and rate limits
- sends a PROXY v2 header so Paper still sees real client IPs

## Deploy

`./deploy.sh` on the proxy host, tunables are consts at the top of `src/main.rs`

## Origin

- firewall the game port so only the proxy IP can reach it
- set `proxies.proxy-protocol: true` in `paper-global.yml`, then full restart
- point the server hostname (A and SRV) at the proxy
