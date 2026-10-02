# www-admin/ — Clearinghouse Admin SPA (ZaarHub)

**Served at:** `https://zaarhub.com/admin/` and `https://directory.swiftsoftware.net/admin/`
**Served from:** `/opt/swift/nginx/www-admin/multidirectory/` (nginx vhost
`/opt/swift/nginx/sites/multi-directory.conf`, `location /admin/ { alias ... }`).

Note the two different slugs — do not "fix" either one:

| Surface | nginx slug | notes |
|---|---|---|
| public site (`/`) | `nginx/www/multi-directory` (hyphen) | **empty and vestigial** — `location /` proxies to `127.0.0.1:8089`, so this root is never read |
| admin console (`/admin/`) | `nginx/www-admin/multidirectory` (no hyphen) | the alias that actually serves |

## Source of truth

* **Public site (`zaarhub.com`, `directory.swiftsoftware.net`): the repo tree
  `frontend/`.** nginx proxies every path to the Multi-Directory Rust service on port 8089,
  which serves the bind-mounted `frontend/` directory (container path
  `/opt/swift/multidirectory-rust/frontend` → host `/opt/swift/apps/multi-directory/frontend`).
  The homepage is `frontend/index.html` read into memory at boot plus a runtime-injected
  `<style id="brand-theme">` (per-network branding from the DB). Edit `frontend/` and restart
  the container; the nginx `www/multi-directory` root is never consulted.
* **Admin console (`/admin/`): THIS repo folder
  (`apps/multi-directory/www-admin/`).** Edit a file here, then **publish it to the live nginx
  directory** `/opt/swift/nginx/www-admin/multidirectory/` with
  `/opt/swift/bin/publish-multidirectory-frontend.sh` (below). Before
  `t_64d6101d` this folder was a hand-kept *snapshot* of the live directory and there was no
  publisher; the publisher closed that gap and the direction is now **repo → live**, matching
  every sibling app (`publish-workflowswift-frontend.sh`, `publish-funnelswift-frontend.sh`, …).

### Publishing an admin-console change (repo → live)

```bash
# 1. see the drift first (changes nothing; exit 1 when anything differs)
/opt/swift/bin/publish-multidirectory-frontend.sh --diff

# 2. publish everything in the served list, or just the files you changed
/opt/swift/bin/publish-multidirectory-frontend.sh
/opt/swift/bin/publish-multidirectory-frontend.sh index.html

# 3. verify the SERVED bytes, not the mtime
curl -s -H 'Host: zaarhub.com' http://127.0.0.1/admin/index.html | sha256sum
sha256sum /opt/swift/nginx/www-admin/multidirectory/index.html   # must match
```

* **Served files — the complete contents of the live root** (four files; nothing else in this
  folder is served):
  `index.html`, `guide-admin.html`, `favicon.svg`, `favicon.ico`.
  `README.md` (this file) lives only in the repo and is deliberately never published.
* **Refusing to clobber a live-only edit.** If the live file differs from the repo copy *and*
  the live path is an uncommitted edit in the frontends repo, the publisher refuses (exit 1) —
  version the live bytes into this folder first, or pass `--force`. The publisher is the only
  intended writer of that root.
* **Rollback copies** of any replaced live file go to
  `/opt/swift/backups/webroot-bak/www-admin/multidirectory/<name>.bak-<stamp>` — *outside* every
  served root (a `*.bak-*` inside a served root is answered by nginx `try_files` at a guessable
  path; see `docs/fleet-webroot-backup-convention.md`).
* **Deploy record.** The live root is itself tracked in the frontends repo
  (`/opt/swift/nginx`, GitHub `Swiftsoftware143/frontends`), so after a publish that repo shows
  the changed bytes and a `record(multi-directory): …` commit is what proves *what is deployed*
  (the favicon change is `0165c37`). This folder is the editable origin; the frontends repo is
  the deployment record.

### The console favicon (`t_7a3e492c`, `t_64d6101d`)

The console is mounted under a **sub-path** of the origin, so a browser's *automatic* icon
request goes to the **origin root** (`https://zaarhub.com/favicon.ico`, the public app's 404),
**not** to `/admin/favicon.ico`. Shipping `favicon.svg`/`favicon.ico` into the console root
alone therefore does not fix anything — `index.html` and `guide-admin.html` each carry the two
`/admin/`-absolute declarations that the browser actually honours:

```html
<link rel="icon" type="image/svg+xml" href="/admin/favicon.svg">
<link rel="alternate icon" href="/admin/favicon.ico">
```

Keep both lines when editing the `<head>` of either page.
