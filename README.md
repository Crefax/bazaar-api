# Bazaar API

Bazaar API is a Rust/Actix service for tracking Hypixel SkyBlock Bazaar market data and serving chart-ready v2 API responses. It stores latest product snapshots, materialized OHLCV candles, user API keys, public access policy, and admin controls.

## Features

- v2-only REST API for products, latest prices, OHLCV candles, and line-series data.
- Built-in admin panel at `/admin`.
- Admin authentication with `ADMIN_API_KEY`, `X-Admin-Api-Key`, HttpOnly sessions, and CSRF protection for cookie-authenticated mutations.
- User API keys with HMAC-hashed storage, one-time secret display, rotate/revoke support, per-key rate limits, and daily quotas.
- Anonymous public access with configurable low rate limits.
- Shared Redis-backed security store for production/multi-instance deployments, with bounded in-memory fallback for local development.
- 15-second tracker cadence with duplicate snapshot skipping.
- MongoDB latest snapshots and materialized candles for fast chart reads.
- Minimal health/readiness endpoints.

## Requirements

- Rust stable
- MongoDB
- Redis for production or any multi-instance deployment

## Configuration

Copy the example env file and replace placeholder secrets:

```powershell
Copy-Item .env.example .env
```

Minimal local run:

```powershell
$env:APP_ENV="development"
$env:ADMIN_API_KEY="replace-with-a-long-random-admin-key"
$env:API_KEY_HASH_PEPPER="replace-with-a-long-random-pepper"
$env:MONGODB_URI="mongodb://localhost:27017"
$env:MONGODB_DB="skyblock"
$env:BIND_ADDR="127.0.0.1:22417"
cargo run
```

Production/multi-instance run:

```powershell
$env:APP_ENV="production"
$env:REQUIRE_REDIS="true"
$env:REDIS_URL="redis://:strong-password@10.0.0.50:6379/0"
$env:CORS_ALLOWED_ORIGINS="https://example.com"
$env:TRUST_PROXY="true"
$env:TRUSTED_PROXY_CIDRS="10.0.0.0/8"
$env:ADMIN_COOKIE_SECURE="true"
cargo run
```

Environment variables:

| Variable | Default | Description |
|---|---:|---|
| `APP_ENV` | `development` | `development` or `production`. Production requires Redis-backed security behavior. |
| `BIND_ADDR` | `127.0.0.1:22417` | HTTP bind address. |
| `MONGODB_URI` | `mongodb://localhost:27017` | MongoDB connection string. |
| `MONGODB_DB` | `skyblock` | MongoDB database name. |
| `ADMIN_API_KEY` | unset | Required for admin panel/API authentication. |
| `API_KEY_HASH_PEPPER` | dev value | HMAC pepper for user API key hashes. Use a strong secret. |
| `REDIS_URL` | unset | Shared Redis URL for rate limits, quotas, sessions, access policy cache, and response cache. |
| `REQUIRE_REDIS` | `false` | Forces fail-closed behavior when Redis is unavailable. Production also requires Redis. |
| `CORS_ALLOWED_ORIGINS` | empty | Public API browser origins. Empty allows any origin only in development. |
| `TRUST_PROXY` | `false` | Enables trusted proxy IP extraction. |
| `TRUSTED_PROXY_CIDRS` | empty | Comma-separated CIDRs allowed to supply `X-Forwarded-For`. |
| `ADMIN_COOKIE_SECURE` | env-based | Defaults to true in production and false in development. |
| `CACHE_MAX_ENTRIES` | `10000` | Bounded local cache entries for development fallback. |
| `RATE_LIMIT_MAX_KEYS` | `50000` | Bounded local rate/quota keys for development fallback. |
| `ADMIN_JSON_LIMIT_BYTES` | `16384` | JSON payload limit. |

## Runtime URLs

- API base: `http://127.0.0.1:22417`
- Admin panel: `GET /admin`
- Health: `GET /health`
- Readiness: `GET /ready`
- OpenAPI summary: `GET /api/v2/openapi.json`

Only `/api/v2/...` is registered. Former `/api/v1/...` and `/api/...` routes are intentionally removed.

## Authentication

Public v2 endpoints support:

```http
X-API-Key: bzusr_...
```

If anonymous access is enabled in the admin access policy, public endpoints can also be called without a key at a lower rate limit.

Admin endpoints require either:

```http
X-Admin-Api-Key: your-admin-key
```

or an admin session cookie from `/admin`. Cookie-authenticated `POST`, `PATCH`, `PUT`, and `DELETE` admin calls also require `X-CSRF-Token`; the built-in panel handles this automatically.

The admin key is rejected when sent as `X-API-Key`.

## Public API v2

| Method | Path | Description |
|---|---|---|
| `GET` | `/api/v2/skyblock/bazaar/products` | List product ids. |
| `GET` | `/api/v2/skyblock/bazaar/products/latest?ids=...` | Get latest data for many products. |
| `GET` | `/api/v2/skyblock/bazaar/products/{product_id}/latest` | Get latest data for one product. |
| `GET` | `/api/v2/skyblock/bazaar/products/{product_id}/candles` | Get OHLCV candles. |
| `GET` | `/api/v2/skyblock/bazaar/products/{product_id}/series` | Get line-series points. |

Supported candle intervals:

```text
15s, 1m, 5m, 15m, 1h, 1d, 1w, 1mo
```

Supported metrics:

```text
buy_price, sell_price, mid_price, spread
```

Supported series stats:

```text
open, high, low, close, avg, volume
```

Granular range limits:

| Interval | Maximum query range |
|---|---:|
| `15s` | 24 hours |
| `1m` | 30 days |
| `5m`, `15m` | 180 days |
| `1h` | 5 years |
| `1d`, `1w`, `1mo` | 100 years |

## Admin API v2

| Method | Path | Description |
|---|---|---|
| `POST` | `/api/v2/admin/session` | Log in and create an admin session cookie. |
| `POST` | `/api/v2/admin/session/logout` | Log out and revoke the admin session. |
| `GET` | `/api/v2/admin/api-keys` | List user API keys. |
| `POST` | `/api/v2/admin/api-keys` | Create a user API key. |
| `PATCH` | `/api/v2/admin/api-keys/{id}` | Update key metadata, status, rate limit, or daily quota. |
| `POST` | `/api/v2/admin/api-keys/{id}/rotate` | Rotate a key and show the new secret once. |
| `DELETE` | `/api/v2/admin/api-keys/{id}` | Revoke a key. |
| `GET` | `/api/v2/admin/access-policy` | Read public API access policy. |
| `PATCH` | `/api/v2/admin/access-policy` | Update anonymous access and default limits. |
| `GET` | `/api/v2/admin/compression/stats` | Read compression statistics. |
| `GET` | `/api/v2/admin/compression/logs` | Read compression logs. |

Create a user key with a daily quota:

```powershell
$body = @{
  name = "Example Client"
  owner_email = "client@example.com"
  rate_limit_per_minute = 600
  daily_quota = 100000
} | ConvertTo-Json

Invoke-RestMethod `
  -Method Post `
  -Uri "http://127.0.0.1:22417/api/v2/admin/api-keys" `
  -Headers @{ "X-Admin-Api-Key" = $env:ADMIN_API_KEY } `
  -ContentType "application/json" `
  -Body $body
```

Use a user key:

```powershell
Invoke-RestMethod `
  -Uri "http://127.0.0.1:22417/api/v2/skyblock/bazaar/products/WHEAT/latest" `
  -Headers @{ "X-API-Key" = "bzusr_..." }
```

## Security Behavior

- Anonymous public rate limit default: `120 req/min/IP`.
- User API key rate limit default: `600 req/min/key`.
- Admin login limits: `5 req/min/IP` and `50 req/hour/IP`.
- Admin API limit: `120 req/min/session-or-admin-key`.
- Daily quotas reset at UTC midnight.
- Production or multi-instance deployments must share the same Redis instance.
- If Redis is required but unavailable, auth/rate-limit/quota/policy operations fail closed with `503`.
- Public `500` responses do not include internal database errors.

## Data Storage

MongoDB collections used by the v2 runtime:

| Collection | Purpose |
|---|---|
| `bazaar` | Raw changed snapshots. |
| `bazaar_latest` | One latest snapshot per product. |
| `bazaar_candles` | Materialized chart candles by product, interval, metric, and period. |
| `api_keys` | Hashed user API keys and per-key limits/quotas. |
| `api_settings` | Public access policy. |
| `compression_log` | Compression/admin statistics support. |

Retention behavior:

| Resolution | Retention |
|---|---|
| Raw 15-second snapshots | 24 hours |
| 15-second candles | 24 hours |
| 1-minute candles | 30 days |
| 5-minute and 15-minute candles | 180 days |
| 1-hour candles | 5 years |
| Daily, weekly, monthly candles | Indefinite |

## Development Checks

```powershell
cargo fmt
cargo check
cargo test
cargo audit
```

## License

This project is licensed under the GNU General Public License v3.0. See [LICENSE](LICENSE) for details.
