#!/usr/bin/env bash
# seo-launch-check.sh — a directory is not "launched" until this passes.
#
#   scripts/seo-launch-check.sh <brand-host> <factory-host> [deep-path]
#
# e.g. scripts/seo-launch-check.sh zaarhub.com directory.swiftsoftware.net /apopka
#
# THE RULE (David, 2026-11-10 — docs/directory-hosting-rule.md):
#   1. every public page canonicalises ABSOLUTELY to the directory's own brand host,
#      no matter which host served it;
#   2. only the brand host advertises a sitemap;
#   3. the factory host's public paths are noindex and its admin keeps working;
#   4. `www.<brand>` answers exactly one 301 to the apex;
#   5. an unmatched path is a real 404 (no soft-404 landing on the SPA shell).
#
# The brand host is `networks.root_domain` for the directory's network. If it is not
# set, the app fails SAFE (no canonical, no sitemap, noindex on the factory host) and
# this check fails — that is the point: a directory with no brand host is not launched.
#
# Exit 0 = every check passed. Exit 1 = at least one failed (list is printed).
set -uo pipefail

BRAND="${1:-}"
FACTORY="${2:-}"
DEEP="${3:-}"

if [ -z "$BRAND" ] || [ -z "$FACTORY" ]; then
    echo "usage: $0 <brand-host> <factory-host> [deep-path]" >&2
    exit 2
fi
BRAND="${BRAND#http://}"; BRAND="${BRAND#https://}"; BRAND="${BRAND%%/*}"
FACTORY="${FACTORY#http://}"; FACTORY="${FACTORY#https://}"; FACTORY="${FACTORY%%/*}"
WWW="www.${BRAND}"

WORK="$(mktemp -d)"
trap 'rm -rf "$WORK"' EXIT

PASSES=0
FAILS=0
pass() { PASSES=$((PASSES + 1)); printf 'PASS  %s\n' "$1"; }
fail() { FAILS=$((FAILS + 1)); printf 'FAIL  %s\n' "$1"; [ $# -gt 1 ] && [ -n "${2:-}" ] && printf '      %s\n' "$2"; }

# curl <outfile> <url> -> prints the final status code (follows redirects)
fetch() {
    local out="$1" url="$2"
    curl -sS -L --max-time 25 -o "$out" -w '%{http_code}' -A 'swiftsoftware-launch-check/1.0' "$url" 2>/dev/null
}
# status only, NO redirect following
status_nofollow() {
    curl -sS -o /dev/null --max-time 25 -w '%{http_code}' -A 'swiftsoftware-launch-check/1.0' "$1" 2>/dev/null
}
canonical_of() {
    # the <link rel="canonical"> href of a saved document
    tr '\n' ' ' < "$1" \
        | grep -oiE '<link[^>]*rel="canonical"[^>]*>' \
        | head -n1 \
        | grep -oiE 'href="[^"]*"' \
        | head -n1 \
        | sed -E 's/^[Hh][Rr][Ee][Ff]="//; s/"$//'
}
canonical_url_expected() {
    # <origin><path>, trailing slash dropped except at the root
    local p="$1"
    if [ "$p" != "/" ]; then p="${p%/}"; fi
    printf 'https://%s%s' "$BRAND" "$p"
}

normalize_path() { local p="$1"; [ "${p:0:1}" = "/" ] || p="/$p"; if [ "$p" != "/" ]; then p="${p%/}"; fi; printf '%s' "$p"; }

echo "== directory launch check =="
echo "   brand host   : $BRAND"
echo "   factory host : $FACTORY"

# ── 0. brand sitemap exists, and every sampled <loc> is on the brand host ─────────
code=$(fetch "$WORK/sitemap.xml" "https://${BRAND}/sitemap.xml")
locs=$(grep -o '<loc>[^<]*</loc>' "$WORK/sitemap.xml" 2>/dev/null | wc -l | tr -d ' ')
if [ "$code" = "200" ] && [ "${locs:-0}" -gt 0 ]; then
    pass "brand sitemap answers 200 (${locs} urls)"
    off=$(grep -o '<loc>[^<]*</loc>' "$WORK/sitemap.xml" | sed -E 's#</?loc>##g' | grep -vc "^https://${BRAND}/")
    if [ "${off:-0}" -eq 0 ]; then
        pass "every sitemap <loc> is on https://${BRAND}"
    else
        fail "sitemap advertises ${off} url(s) on another host" "$(grep -o '<loc>[^<]*</loc>' "$WORK/sitemap.xml" | sed -E 's#</?loc>##g' | grep -v "^https://${BRAND}/" | head -n3 | tr '\n' ' ')"
    fi
else
    fail "brand sitemap missing or empty (HTTP $code, ${locs:-0} urls)"
fi

# ── 1. deep path: default to the first non-root url in the brand sitemap ──────────
if [ -z "$DEEP" ]; then
    DEEP=$(grep -o '<loc>[^<]*</loc>' "$WORK/sitemap.xml" 2>/dev/null \
        | sed -E 's#</?loc>##g' | sed -E "s#^https://${BRAND}##" | grep -v '^/$' | head -n1)
    [ -n "$DEEP" ] && echo "   deep path    : $DEEP (from the brand sitemap)"
else
    echo "   deep path    : $DEEP"
fi
DEEP=$(normalize_path "${DEEP:-/}")

# ── 2. identical ABSOLUTE canonical on every host, for / and the deep path ───────
for path in "/" "$DEEP"; do
    [ "$path" = "/" ] && label="/" || label="$path"
    want=$(canonical_url_expected "$path")
    for host in "$BRAND" "$WWW" "$FACTORY"; do
        code=$(fetch "$WORK/page.html" "https://${host}${path}")
        got=$(canonical_of "$WORK/page.html")
        if [ "$code" = "200" ] && [ "$got" = "$want" ]; then
            pass "${host}${label} canonical = ${got}"
        elif [ -z "$got" ]; then
            fail "${host}${label} emits NO canonical (HTTP $code)" "expected ${want} — is networks.root_domain set?"
        else
            fail "${host}${label} canonical = '${got}'" "expected ${want} (HTTP $code)"
        fi
    done
done

# ── 3. exactly one advertised sitemap, on the brand host only ────────────────────
fetch "$WORK/robots-brand.txt" "https://${BRAND}/robots.txt" > /dev/null
brand_lines=$(grep -ciE '^[[:space:]]*sitemap:' "$WORK/robots-brand.txt" 2>/dev/null)
brand_target=$(grep -iE '^[[:space:]]*sitemap:' "$WORK/robots-brand.txt" 2>/dev/null | head -n1 | sed -E 's/^[Ss]itemap:[[:space:]]*//; s/[[:space:]]*$//')
if [ "${brand_lines:-0}" = "1" ] && [ "$brand_target" = "https://${BRAND}/sitemap.xml" ]; then
    pass "${BRAND} robots.txt advertises exactly one sitemap (${brand_target})"
else
    fail "${BRAND} robots.txt advertises ${brand_lines:-0} sitemap line(s)" "first='${brand_target}' expected=https://${BRAND}/sitemap.xml"
fi
fetch "$WORK/robots-factory.txt" "https://${FACTORY}/robots.txt" > /dev/null
factory_lines=$(grep -ciE '^[[:space:]]*sitemap:' "$WORK/robots-factory.txt" 2>/dev/null)
if [ "${factory_lines:-0}" = "0" ]; then
    pass "${FACTORY} robots.txt advertises NO sitemap"
else
    fail "${FACTORY} robots.txt still advertises a sitemap (${factory_lines})"
fi
fetch "$WORK/robots-www.txt" "https://${WWW}/robots.txt" > /dev/null
www_lines=$(grep -ciE '^[[:space:]]*sitemap:' "$WORK/robots-www.txt" 2>/dev/null)
[ "${www_lines:-0}" = "1" ] \
    && pass "${WWW} robots.txt (via the 301) resolves to the same single sitemap" \
    || fail "${WWW} robots.txt advertises ${www_lines:-0} sitemap line(s)"
for host in "$BRAND" "$FACTORY"; do
    grep -qiE '^[[:space:]]*disallow:[[:space:]]*/admin/' "$WORK/robots-$( [ "$host" = "$BRAND" ] && echo brand || echo factory).txt" \
        && pass "${host} robots.txt still disallows /admin/" \
        || fail "${host} robots.txt lost its /admin/ disallow"
done

# ── 4. an unmatched path is a real 404, on both hosts ────────────────────────────
UNMATCHED="/launch-check-does-not-exist-$$"
for host in "$BRAND" "$FACTORY"; do
    code=$(status_nofollow "https://${host}${UNMATCHED}")
    pass_hu=$(fetch "$WORK/404.html" "https://${host}${UNMATCHED}")
    can=$(canonical_of "$WORK/404.html")
    if [ "$code" = "404" ] && [ "$pass_hu" = "404" ]; then
        pass "${host}${UNMATCHED} is a real 404 (no redirect, no SPA shell)"
    else
        fail "${host}${UNMATCHED} answered ${code}/${pass_hu}, expected 404" "canonical='${can}'"
    fi
done

# ── 5. www answers exactly ONE 301 to the apex ───────────────────────────────────
www_header=$(curl -sS -o /dev/null -D - --max-time 25 -A 'swiftsoftware-launch-check/1.0' "https://${WWW}${DEEP}" 2>/dev/null)
www_code=$(printf '%s' "$www_header" | head -n1 | awk '{print $2}')
www_location=$(printf '%s' "$www_header" | grep -i '^location:' | head -n1 | sed -E 's/^[Ll]ocation:[[:space:]]*//; s/\r$//')
case "$www_location" in
    "https://${BRAND}"*) ok_loc=1 ;;
    *) ok_loc=0 ;;
esac
if [ "$www_code" = "301" ] && [ "$ok_loc" = "1" ]; then
    pass "${WWW}${DEEP} -> 301 ${www_location}"
else
    fail "${WWW}${DEEP} answered ${www_code:-?} Location='${www_location}'" "expected one 301 to https://${BRAND}/…"
fi

# ── 6. the factory host's PUBLIC pages are noindex; the ADMIN is untouched ──────
hdr=$(curl -sS -o /dev/null -D - --max-time 25 -A 'swiftsoftware-launch-check/1.0' "https://${FACTORY}${DEEP}" 2>/dev/null)
if printf '%s' "$hdr" | grep -qi '^x-robots-tag:.*noindex'; then
    pass "${FACTORY}${DEEP} carries X-Robots-Tag: noindex (public path only)"
else
    fail "${FACTORY}${DEEP} has no X-Robots-Tag: noindex — the factory host could be indexed"
fi
login_code=$(status_nofollow "https://${FACTORY}/admin/login")
fetch "$WORK/login.html" "https://${FACTORY}/admin/login" > /dev/null
if [ "$login_code" = "200" ] && grep -qi 'type="password"' "$WORK/login.html"; then
    pass "${FACTORY}/admin/login answers 200 with the console sign-in form (factory admin reachable)"
else
    fail "${FACTORY}/admin/login answered ${login_code:-?} without the sign-in form — admin console broken"
fi
# The login PAGE is deliberately noindexed by the vhost (it is a sign-in form, not
# content) — what must stay untouched is the API the admin SPA calls.
api_hdr=$(curl -sS -o /dev/null -D - --max-time 25 -A 'swiftsoftware-launch-check/1.0' "https://${FACTORY}/api/v1/health" 2>/dev/null)
if printf '%s' "$api_hdr" | grep -qi '^x-robots-tag:'; then
    fail "${FACTORY}/api/v1/health carries X-Robots-Tag — the admin API must be left exactly as it was"
else
    pass "${FACTORY}/api/v1/health carries no X-Robots-Tag (admin API untouched)"
fi

echo
echo "== ${PASSES} passed, ${FAILS} failed =="
[ "$FAILS" -eq 0 ] || exit 1
echo "LAUNCHED: ${BRAND} is the one and only ranked host for this directory."
exit 0
