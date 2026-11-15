# Docker Compose Deployment

Production deployment for the Hyphen block explorer.

## Architecture

Five containers run across six Docker networks. Four are internal;
`net-edge` carries the published ports and `net-p2p` gives the Core node
its outbound peer-to-peer connections. Only the nginx container exposes
ports to the host.

| Network    | Members           | Purpose                        |
| ---------- | ----------------- | ------------------------------ |
| `net-edge` | nginx             | Published HTTP and HTTPS ports |
| `net-p2p`  | dashd             | Core peer-to-peer connections  |
| `net-dash` | dashd, hyphend    | Core node RPC and ZMQ          |
| `net-db`   | postgres, hyphend | Database access                |
| `net-api`  | hyphend, nginx    | API reverse proxy              |
| `net-web`  | webui, nginx      | Frontend reverse proxy         |

The nginx container applies rate limiting and security headers, and
proxies requests to the appropriate backend. hyphend sets the CORS headers
itself. The webui is a static SvelteKit build served by an internal nginx
instance (`nginx/static.conf`), together with its runtime configuration at
`/webui.conf`; API calls from the browser reach hyphend through the public
URL via the proxy.

## Requirements

Hyphen indexes the whole chain into PostgreSQL. On mainnet the database
grows to about 430 GB and the Core node with `txindex` needs about 60 GB
more; testnet needs about 40 GB in all. The first sync is bounded by
database I/O, so local SSD or NVMe storage and enough memory to cache
PostgreSQL's working set make a large difference. PostgreSQL starts with
its small default settings; the commented `command` on the postgres service
shows the ones to raise for a full mainnet sync.

## Nginx configuration

The nginx configuration is split to avoid duplication between
environments. All proxy rules, rate limiting, security headers, and bot
protection live in shared snippets included by both the local and
production configs:

| File                        | Purpose                                             |
| --------------------------- | --------------------------------------------------- |
| `nginx/include/http.conf`   | Rate limit zones, bot map, log format (http-level)  |
| `nginx/include/server.conf` | Headers, compression, locations (server-level)      |
| `nginx/hyphen.conf`         | Local: plaintext HTTP, includes shared snippets     |
| `nginx/tls.conf`            | Production: adds TLS via SWAG, HSTS, HTTPS redirect |
| `nginx/static.conf`         | Webui container: static files and `/webui.conf`     |

Editing the shared includes changes behaviour in both environments.
The only differences between local and production are TLS termination,
HSTS and the HTTPS redirect.

## Production setup

Run every command below from this directory, `contrib/deploy`.

1. **Configure the domain.**

   Copy the example environment file and set the public domain and contact
   email used for TLS certificate issuance, the PostgreSQL password, and
   the network:

   ```sh
   cp .env.example .env
   ```

   ```env
   HYPHEN_DOMAIN=explorer.yourdomain.com
   HYPHEN_EMAIL=admin@yourdomain.com
   POSTGRES_PASSWORD=change-me
   HYPHEN_NETWORK=mainnet
   ```

   `docker-compose.yml` interpolates these values into the containers.

2. **Create the configuration files.**

   ```sh
   cp dash.conf.example dash.conf
   cp hyphen.conf.example hyphen.conf
   ```

   Change the RPC credentials in both files so they match, and put the
   PostgreSQL password from `.env` into the `[db.client]` URL in
   `hyphen.conf`. Do this before the first start: PostgreSQL sets its
   password only when it creates its data volume. `.env`, `dash.conf` and
   `hyphen.conf` are gitignored.

   Core accepts RPC from any address (`rpcallowip=0.0.0.0/0`). That relies on
   its ports staying unpublished and on the RPC password; keep both that way.

3. **Build and start:**

   ```sh
   docker compose up -d --build
   ```

   The `--build` flag rebuilds the `hyphend` and `webui` images from the
   current repository checkout so that the deployed version always reflects
   the working tree. SWAG handles TLS certificate issuance automatically.

4. **Verify.**

   ```sh
   docker compose logs -f hyphend
   docker compose logs -f dashd
   ```

## Local development

`docker-compose.local.yml` runs the stack locally without TLS. It
replaces the SWAG container with plain nginx using `nginx/hyphen.conf`,
which includes the same shared snippets as production. Create `.env`,
`dash.conf` and `hyphen.conf` as above, then from this directory:

```sh
docker compose -f docker-compose.yml -f docker-compose.local.yml up -d --build
```

The explorer is available at `http://localhost` with the API at
`http://localhost/api/` and directly at `http://127.0.0.1:3080`.

## Testnet

The files default to mainnet. For testnet:

- `dash.conf`: add `testnet=1` at the top, and move `rpcbind=0.0.0.0` under
  a `[test]` section at the end, since Core reads that option per network.
- `hyphen.conf`: point both Core URLs at port `19998`, and use the testnet
  values noted above `[db.layout]` and `chainlock_start_block = 78800`.
- `.env`: set `HYPHEN_NETWORK=testnet`, which serves `webui/testnet.conf`.

The `[db.layout]` values are recorded on first start, so a database synced
for one network cannot be reused for the other.

## Rebuilding after code changes

```sh
docker compose build hyphend webui
docker compose up -d hyphend webui
```

## Configuration reference

| File                      | Mounted at                           | Format |
| ------------------------- | ------------------------------------ | ------ |
| `hyphen.conf`             | hyphend `/data/hyphen.conf`          | TOML   |
| `dash.conf`               | dashd `/home/dash/.dashcore/dash.conf` | INI  |
| `webui/<network>.conf`    | webui `/etc/hyphen/webui.conf`       | TOML   |
| `nginx/static.conf`       | webui `/etc/nginx/conf.d/default.conf` | nginx |
| `nginx/`                  | `/etc/nginx/hyphen/` (both profiles) | nginx  |
| `.env`                    | Interpolated by `docker-compose.yml` |        |

## Network segmentation

Each container is attached only to the networks it requires:

- **dashd** is on `net-dash`, where only hyphend can reach it, and on
  `net-p2p`, which gives Core outbound access to its peers.
- **postgres** is on `net-db` and can only communicate with hyphend.
- **hyphend** is on `net-dash`, `net-db`, and `net-api`; it reaches the
  Core node, the database, and is reachable by nginx for API proxying.
- **webui** is on `net-web` and can only communicate with nginx.
- **nginx** is on `net-api` and `net-web`, and on `net-edge` for the
  host-exposed ports that carry external traffic.
