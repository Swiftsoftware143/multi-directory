# MEMORY.md — Builder Agent

## Miami VPS
- **Provider:** ReliableSite
- **IP:** 209.222.97.179
- **CPU:** Intel Xeon E3-1240 v5 (4c/8t)
- **RAM:** 64 GB DDR4
- **Storage:** 2x2TB SSD
- **OS:** Ubuntu 24.04 LTS

## Multi-Directory Platform (Primary)
Multi-Directory is the white-label directory engine. zaarhub.com is the flagship deployment.
- **Repo:** /opt/swift/apps/multi-directory/
- **Binary:** /opt/swift/apps/multi-directory
- **Service:** multidirectory.service (port 8089)
- **DB:** multi_directory

## Beyond Multi-Directory — Cross-App Work
Builder is NOT limited to Multi-Directory. You are a general-purpose VPS Vibe Engineer who works across ALL 7 apps:

| App | Port | What you've built there |
|-----|------|------------------------|
| Multi-Directory | 8089 | Platform, zaarhub.com, scanner, blog, B2B marketplace, content engine |
| CoreSwift CRM | 8084 | Portfolio sync broadcast module, admin dashboards |
| FunnelSwift | 8080 | Portfolio sync receiver, admin SPA fixes |
| IncentiveSwift | 8083 | Loyalty proxy (loyalty_proxy.rs), portfolio sync, admin fixes |
| WorkflowSwift | 8085 | Portfolio sync receiver, admin dashboard |
| ADASwift | 8087 | Port SSR + legal pages to Multi-Directory, admin UI reference |
| MissedCall | 8088 | Portfolio sync receiver |

**Key cross-app projects:**
- Portfolio sync broadcast — CoreSwift CRM → all 5 sister apps (sub-agent session e90e6d32)
- Admin dashboard audit across 6 apps
- Nginx 3-layer routing verification
- Fixes: directories→tenants table refs, sqlx::query!→query_as across multiple handlers

## Fleet (Miami VPS)
| Bot | Title | Telegram |
|-----|-------|----------|
| SwiftSoftware CEO Bot | Lead Architect | @swiftsoftware_ceo_bot |
| Prime | Lead Vibe Engineer | @swift_reliable_vps_bot |
| Builds | Vibe Engineer — Build/Deploy | @builds_reliable_bot |
| Automation | Vibe Engineer — Workflows | @automation_reliable_bot |
| Monitoring | Vibe Engineer — Health | @monitoring_reliable_bot |
| Hermes SwiftImpact | Vibe Engineer — Marketing | @hermes_swiftimpact_bot |
| Builder (self) | Vibe Engineer — Cross-App + Platform | — |

## Tools & Access
- **Postgres:** postgres://swift:***@localhost:5432 (7 databases)
- **n8n:** admin@swiftsoftware.com / SwiftAdmin2026!
- **All app repos:** /opt/swift/apps/
- **Nginx:** /opt/swift/nginx/

## Build Lock Protocol
- ALWAYS: /opt/swift/build-lock.sh <app> cargo build --release
- Exit 2 = another bot building. Wait 30s, retry once.

## Active Tasks
- Scanner phone testing (assigned by Lead Architect)
- Loyalty display verification on phone
- Cross-app portfolio sync verification after migration

## Recent Work
- Scanner PWA at zaarhub.com/scanner
- B2B marketplace: RFQ, lead sharing, co-op buying
- Content engine: blog, SEO, research, AEO scoring
- Portfolio sync broadcast across 6 apps
- Agent rules: read-only protection, admin login verification
- directories→tenants table ref fix (4 handlers)
