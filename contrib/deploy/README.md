# Docker Compose Deployment

Production deployment for the Hyphen block explorer.

## Architecture

Five containers run across four isolated internal Docker networks.
Only the nginx container exposes ports to the host.

| Network    | Members           | Purpose                |
| ---------- | ----------------- | ---------------------- |
| `net-dash` | dashd, hyphend    | Core node RPC and ZMQ  |
| `net-db`   | postgres, hyphend | Database access         |
| `net-api`  | hyphend, nginx    | API reverse proxy       |
| `net-web`  | webui, nginx      | Frontend reverse proxy  |

The nginx container enforces CORS, applies rate limiting, and proxies
requests to the appropriate backend. The webui is a static SvelteKit
build served by an internal nginx instance; API calls from the browser
reach hyphend through the public URL via the proxy.

## Nginx configuration

The nginx configuration is split to avoid duplication between
environments. All proxy rules, rate limiting, security headers, and bot
protection live in shared snippets included by both the local and
production configs:

| File                    | Purpose                                       |
| ----------------------- | --------------------------------------------- |
| `nginx/include/http.conf`   | Rate limit zones, bot map, log format (http-level)  |
| `nginx/include/server.conf` | Headers, compression, locations (server-level)      |
| `nginx/hyphen.conf`         | Local: plaintext HTTP, includes shared snippets     |
| `nginx/tls.conf`            | Production: adds TLS via SWAG, HSTS, HTTPS redirect |

Editing the shared includes changes behaviour in both environments.
The only differences between local and production are TLS termination,
HSTS, the HTTPS redirect, and the CORS origin scheme.

## Production setup

1. **Configure the domain.**

   Copy the example environment file and set the public domain and contact
   email used for TLS certificate issuance:

   ```sh
   cp .env.example .env
   ```

   ```env
   HYPHEN_DOMAIN=explorer.yourdomain.com
   HYPHEN_EMAIL=admin@yourdomain.com
   ```

   This file is the single source of truth for the deployment domain;
   `docker-compose.yml` interpolates its values into the SWAG container.

2. **Review configuration files.**

   `dash.conf` and `hyphen.conf` contain skeleton defaults. At minimum,
   change the RPC credentials in both files so they match, and review the
   PostgreSQL password in `hyphen.conf`.

   All three files (`.env`, `dash.conf`, `hyphen.conf`) are gitignored after
   the initial commit; local edits are not tracked.

3. **Remove the override file** if present, then build and start:

   ```sh
   rm -f docker-compose.override.yml
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

A `docker-compose.override.yml` is provided for running the stack locally
without TLS. It replaces the SWAG container with plain nginx using
`nginx/hyphen.conf`, which includes the same shared snippets as
production.

```sh
cp .env.local .env
docker compose up -d --build
```

Docker Compose merges the override file automatically when it sits
alongside `docker-compose.yml`. The explorer is available at
`http://localhost` with the API at `http://localhost/api/` and directly
at `http://localhost:3080`.

## Rebuilding after code changes

```sh
docker compose build hyphend webui
docker compose up -d hyphend webui
```

## Configuration reference

| File                      | Mounted at                           | Format |
| ------------------------- | ------------------------------------ | ------ |
| `hyphen.conf`             | hyphend `/data/hyphen.conf`          | TOML   |
| `dash.conf`               | dashd `/dash/.dashcore/dash.conf`    | INI    |
| `nginx/`                  | `/etc/nginx/hyphen/` (both profiles) | nginx  |
| `.env`                    | Interpolated by `docker-compose.yml` |        |

## Network segmentation

Each container is attached only to the networks it requires:

- **dashd** is on `net-dash` and can only communicate with hyphend.
- **postgres** is on `net-db` and can only communicate with hyphend.
- **hyphend** is on `net-dash`, `net-db`, and `net-api`; it reaches the
  Core node, the database, and is reachable by nginx for API proxying.
- **webui** is on `net-web` and can only communicate with nginx.
- **nginx** is on `net-api` and `net-web` with host-exposed ports for
  external traffic.
