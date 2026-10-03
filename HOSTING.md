# Hosting & Handover — Multi-Directory (ZaarHub platform)

This checkout is the **complete product**, not a ZaarHub-only build. It runs on the operator's
own server exactly as it runs on ours; the difference between any two directories is
configuration and data, never code (see `docs/ZAARHUB-LEDGER.md` for the full
TEMPLATE / CONFIGURABLE / CUSTOM classification).

Card: **B85 — handover readiness**.

---

## 1. What you need

| | Required | Notes |
|---|---|---|
| OS | Ubuntu 22.04+ or Debian 12 (x86-64) | anything with glibc works; the binary is statically linked except for libssl/libc |
| Postgres | 14–16 | the directory owns one database; nothing else is shared |
| Rust | 1.80+ | only to **build**; not needed at runtime if you ship the binary |
| Redis | optional | used for caching when `REDIS_URL` points somewhere |
| Web server | nginx or Caddy | terminates TLS and proxies to the app on `127.0.0.1:8089` |

## 2. Bare metal (the path verified on 2026-10-03)

```bash
git clone <your-copy-of-this-repo> /srv/multidirectory
cd /srv/multidirectory
sudo bash deploy/install.sh \
    --database-url 'postgres://md:secret@127.0.0.1:5432/multi_directory' \
    --admin-email   'you@example.com' \
    --systemd
```

`install.sh` is idempotent and does four things:

1. writes `.env` (mode 0600) with a random `JWT_SECRET` and `PROVIDER_KEY_ENC_SECRET`;
2. **builds the schema from nothing** — `src/migrations/000_baseline_live_schema.sql` first, then
   the deltas, the same path the app uses at boot;
3. creates your **first admin login** (see §4 — a fresh database has *no* user row);
4. optionally installs a systemd unit and starts the service.

Verify it came up:

```bash
curl -sf http://127.0.0.1:8089/api/v1/health
# -> {"status":"ok", ...}
```

> **Health-endpoint gotcha:** `/api/health` answers HTTP 200 with SPA HTML. Always probe
> **`/api/v1/health`** — that is the real readiness endpoint.

## 3. Docker

```bash
cp deploy/.env.example .env      # edit the CHANGE_ME values
docker compose -f deploy/docker-compose.yml up -d --build
docker compose -f deploy/docker-compose.yml exec app \
    /app/multidirectory create-admin --email you@example.com
```

The compose file brings up Postgres + the app with a persistent volume for the database and one
for uploads. The image is built from this checkout (`deploy/Dockerfile`).

*The compose file's syntax is validated in CI; the image build itself is exercised on the
operator's machine, not on the authoring host.*

## 4. First login — `create-admin`

**A freshly-migrated database has zero rows in `users`.** That is deliberate: no default
password ships. Create the first account:

```bash
# generates a 24-character password and prints it exactly once
./target/release/multidirectory create-admin --email you@example.com

# or pin your own
./target/release/multidirectory create-admin --email you@example.com --password 'choose-a-strong-one'

# attach to a tenant other than the default `swiftsoftware`
./target/release/multidirectory create-admin --email you@example.com --tenant my-directory
```

* the password is stored as an **Argon2** hash — the plaintext is never written anywhere;
* re-running refuses to touch an existing account unless you pass `--force`;
* the command needs only `DATABASE_URL` — not `JWT_SECRET`, not a running app.

Then log in at `/admin-login.html` (the console itself is `/admin-panel.html`).

## 5. Where settings live

| Layer | Holds | Changed by |
|---|---|---|
| `.env` | infrastructure only: database URL, JWT secret, encryption master key, port, base domain | you, once |
| **Admin panel** | everything a directory *is*: branding, directories & networks, categories, provider API keys, payment gateways, email/SMTP, loyalty programmes, homepage, legal pages, plans, ad zones | the operator, in the browser — **no SQL, no shell** |

Provider credentials (Google Places, Mailgun, Stripe, OpenAI…) live in the **provider_keys**
table, encrypted at rest, entered per directory/network in the admin panel. Nothing is baked
into the code and nothing is a server-wide environment variable.

## 6. Backups

Everything worth keeping is in Postgres plus the uploads directory.

```bash
# database
pg_dump "$DATABASE_URL" -Fc -f multidirectory-$(date +%F).dump
# uploads
tar czf uploads-$(date +%F).tar.gz "$MD_UPLOADS_ROOT"
```

Restore is `pg_restore -d "$DATABASE_URL" --clean multidirectory-<date>.dump`. The schema is
then already up to date with whatever binary you run — the app applies any missing migration
at boot.

## 7. Upgrades

```bash
sudo bash deploy/upgrade.sh        # git pull + cargo build --release + migrate + restart
```

Migrations are additive and idempotent; never edit an applied file — a change is always a new
higher-numbered file in `src/migrations/`.

## 8. Troubleshooting

* **Deep links show stale content but the homepage is fine** — `index.html` is read into memory
  once at boot; restart the service after editing it.
* **Provider key saves fail** — `PROVIDER_KEY_ENC_SECRET` is missing or shorter than 32 chars.
  Key writes fail closed rather than storing plaintext; fix the env and restart.
* **The directory is unreachable at `/<city>`** — the slug collides with a top-level route.
  Startup logs the clash and `GET /api/v1/seo/subfolder-clashes` lists them; the directory is
  still served at `/d/<slug>`.
* **Logins return 401 on a site that clearly has users** — you are on a different `BASE_DOMAIN`
  or a different database than the one you seeded.

## 9. What was custom to ZaarHub

Read `docs/ZAARHUB-LEDGER.md`. The short version: ZaarHub's own data, copy, categories and
choices are ZaarHub's; the mechanisms are all template code that ships in this checkout.
