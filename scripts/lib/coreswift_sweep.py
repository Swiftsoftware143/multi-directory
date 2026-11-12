"""CoreSwift contact-mirror sweep — shared by every claim-driving harness.

A business claim fans the owner out to CoreSwift (`coreswift::push_claimed_business` plus the
`tag_sync` follower it spawns) as a contact on the directory's tenant. Nothing in
multi_directory's own teardown touches that row, so a harness that POSTs
`/api/v1/businesses/:id/claim` against a listing that HAS a directory leaves a permanent CRM
contact behind on every run (measured 2026-11-12: 15 repo harnesses + 2 /opt proofs, one contact
each; a listing with `directory_id IS NULL` is skipped by the push, which is why the b2b-register
harnesses never leaked).

The push runs in a detached tokio task whose HTTP calls time out at 10s, so a single DELETE can
run before the contact lands — poll across a window in teardown.

Order matters: `tag_assignments` is a polymorphic (entity_type, entity_id) join with NO FK
cascade, so it is removed first; the contact delete then cascades list_members, scores, tickets,
contact_field_values and link_clicks.

Every prefix is a fixture-address prefix owned by one harness (`zz-review-owner-`, ...); nothing
outside the reserved/fixture domains is ever matched.
"""

import subprocess
import time

PSQL = ["/opt/swift/bin/swift-psql.sh", "coreswift_crm", "-t", "-A", "-c"]


def cssql(query, timeout=40):
    """Run one statement against CoreSwift (coreswift_crm)."""
    r = subprocess.run(PSQL + [query], capture_output=True, text=True, timeout=timeout)
    if r.returncode != 0:
        raise RuntimeError(f"coreswift psql failed: {r.stderr.strip()[:300]}")
    return [ln for ln in r.stdout.strip().splitlines() if ln]


def _where(prefixes):
    return " OR ".join("email LIKE '%s%%'" % p for p in prefixes)


def count_coreswift_mirror(prefixes):
    if isinstance(prefixes, str):
        prefixes = [prefixes]
    rows = cssql("SELECT count(*) FROM contacts WHERE %s;" % _where(prefixes))
    return int(rows[0]) if rows else -1


def sweep_coreswift_mirror(prefixes, window=0):
    """Delete the CoreSwift contact mirror for these fixture prefixes; return what is left.

    Pass `window` (seconds) in teardown: the claim's cross-app push is fire-and-forget, so a
    single delete can beat the contact to the table.
    """
    if isinstance(prefixes, str):
        prefixes = [prefixes]
    where = _where(prefixes)
    deadline = time.time() + window
    while True:
        cssql(
            "DELETE FROM tag_assignments WHERE entity_type='contact' AND entity_id IN "
            "(SELECT id FROM contacts WHERE %s);" % where
        )
        cssql("DELETE FROM contacts WHERE %s;" % where)
        if time.time() >= deadline:
            break
        time.sleep(2)
    return count_coreswift_mirror(prefixes)
