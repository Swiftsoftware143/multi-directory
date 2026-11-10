# Directory hosting rule — one brand host per directory

Source: David, relayed by Muse on the agent bus, 2026-11-10.
Status: **STANDING RULE** — every directory the Multi-Directory factory creates, current and future.
Fleet copy (same rule, operations view): `/opt/swift/docs/fleet-directory-brand-host-rule-2026-11-10.md`.

## The rule

1. Every directory gets its **own brand host** (ZaarHub = `zaarhub.com`).
2. That brand host is the **only** host that ranks and gets indexed.
3. Public pages **canonicalise absolutely** to the brand host, no matter which host served them:
   `<link rel="canonical" href="https://<brand-host>/<path>">`, `og:url` and `twitter:url` the same.
4. Only the brand host **advertises a sitemap** (one `Sitemap:` line in its `robots.txt`).
5. The factory subdomain (`directory.swiftsoftware.net`) is the factory's **home**: its public
   pages are `X-Robots-Tag: noindex` and it never advertises a sitemap. It must never appear in
   search results.
6. The factory **admin must keep working** on that subdomain: `/admin/`, `/admin-panel`, admin
   static assets and every API path the admin SPA calls stay exactly as they are.

## Where the brand host lives

`networks.root_domain` — the directory business's own domain, in **data**, not in code. The app
reads it at boot and refreshes it every 60 s (`src/seo_canonical.rs`), and `PUT
/api/v1/networks/<id>/root-domain` (or Directory Details in the panel) publishes a new value at
once. There is no compile-time canonical origin and no host allowlist any more.

A network row with `root_domain` NULL is a **directory that has not been given a host yet**.

## Fail safe, not fail open

A directory with no brand host set is **non-indexable by default**:

- public pages on the factory host carry `X-Robots-Tag: noindex, follow`;
- no `Sitemap:` line is advertised on any host;
- no canonical is emitted at all — and never a canonical pointing at another directory's brand.

So forgetting the field produces "not indexed", never "indexed on three hostnames". The admin is
unaffected: `/admin*` and `/api/` are exempt from the noindex.

## The launch check

A directory is **not launched** until this passes on its live hosts:

```bash
scripts/seo-launch-check.sh <brand-host> <factory-host> [deep-path]
# e.g.
scripts/seo-launch-check.sh zaarhub.com directory.swiftsoftware.net /apopka
```

It answers pass/fail on: absolute canonical identical on every host (brand, `www`, factory) for
`/` and one deep path; exactly one real 404 for an unmatched path; exactly one advertised sitemap
(brand host only); `www.<brand>` is one 301 to the apex; the factory's public path is noindex; the
factory admin login page is 200 and *not* noindexed. Exit 0 means launched.

The deep path defaults to the first non-root URL in the brand host's own sitemap.

## Checklist when creating a directory

1. Create the network + directory as usual.
2. **Set the brand host**: Directory Details → root domain, i.e.
   `PUT /api/v1/networks/<network-id>/root-domain {"root_domain":"<domain>"}`.
   It must be a bare domain, e.g. `zaarhub.com` (no scheme, no `www`).
3. Point DNS + nginx at the app (`zaarhub.com` and `www.zaarhub.com`, the latter 301 to the apex)
   and purge any cached `robots.txt` at the edge.
4. Run `scripts/seo-launch-check.sh <brand-host> directory.swiftsoftware.net <deep-path>`.
5. Only then is the directory launched. If the check fails because the canonical is missing, the
   brand host is not set — that is the fail-safe doing its job, not a bug.

## Why this is a rule and not a one-off fix

The factory is a multi-directory engine: one app, one nginx vhost, many hostnames. Any host that
serves the same rendered page will self-canonicalise it unless the canonical is pinned to the
directory's own brand host, so every new directory would otherwise repeat ZaarHub's Search Console
failure (8 "Duplicate without user-selected canonical", 6 soft-404s, three competing hosts) and be
fixed after the fact. ZaarHub's own instance of this rule is card `t_f97664fe`; the factory-level
enforcement (this document + the fail-safe + the script) is card `t_0b3cce4d`.
