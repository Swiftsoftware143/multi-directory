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
* **Admin console (`/admin/`): the live nginx directory
  `/opt/swift/nginx/www-admin/multidirectory/`.** This repo folder is a **versioned snapshot**
  of that directory (there is no publisher — files reach the served root by hand-`cp`, see
  commit `d249284`, which versioned `index.html`; `guide-admin.html` was versioned by
  `t_8a811ee2`).

### Publishing an admin-console change

There is no publisher for this path, so a repo edit does **not** deploy by itself. After
changing a file here:

```bash
cp /opt/swift/apps/multi-directory/www-admin/<file> \
   /opt/swift/nginx/www-admin/multidirectory/<file>
# then verify the served bytes, not the mtime:
curl -s -H 'Host: zaarhub.com' http://127.0.0.1/admin/<file> | md5sum
md5sum /opt/swift/nginx/www-admin/multidirectory/<file>
```

A push to this folder is a *snapshot of what is live*, so keep the two in sync in the same
change. (Adding a real publisher for this path is a known gap — not yet built.)
