# Bazaar API

Bazaar API is a Rust/Actix service for tracking Hypixel SkyBlock Bazaar market data and serving chart-ready API responses. It stores the latest product state, materialized OHLCV candles, and public/admin API access controls.

## Features

- Chart-focused v2 REST API for products, latest prices, OHLCV candles, and line-series data.
- Built-in admin panel at `/admin`.
- Admin authentication with `ADMIN_API_KEY`, `X-Admin-Api-Key`, and HttpOnly admin sessions.
- User API keys with hashed storage, per-key rate limits, rotate/revoke support, and `X-API-Key` authentication.
- Anonymous public access with configurable rate limits.
- 15-second tracker cadence with duplicate snapshot skipping.
- MongoDB-backed latest snapshots and materialized candles.
- In-process cache and rate-limit fallback.
- Health/readiness endpoints.

## Requirements

- Rust stable
- MongoDB running locally or reachable over the network

## Configuration

Copy the example env file and replace placeholder values:

```powershell
Copy-Item .env.example .env
```

The application reads environment variables directly. In PowerShell, set them before running:

```powershell
$env:ADMIN_API_KEY="replace-with-a-long-random-admin-key"
$env:MONGODB_URI="mongodb://localhost:27017"
$env:MONGODB_DB="skyblock"
$env:BIND_ADDR="127.0.0.1:22417"
cargo run
```

Available environment variables:

| Variable | Default | Description |
|---|---:|---|
| `BIND_ADDR` | `127.0.0.1:22417` | HTTP bind address. |
| `MONGODB_URI` | `mongodb://localhost:27017` | MongoDB connection string. |
| `MONGODB_DB` | `skyblock` | MongoDB database name. |
| `ADMIN_API_KEY` | unset | Required for admin panel/API authentication. |
| `CORS_ALLOWED_ORIGINS` | empty | Comma-separated allowed origins. Empty means allow any origin for local/dev use. |
| `TRUST_PROXY` | `false` | Trust `X-Forwarded-For` for rate-limit IPs. Enable only behind a trusted proxy. |
| `REDIS_URL` | unset | Reserved for optional Redis. Current runtime uses in-process fallback when Redis is not wired. |

## Running

```powershell
cargo run
```

Default URLs:

- API base: `http://127.0.0.1:22417`
- Admin panel: `http://127.0.0.1:22417/admin`
- Health: `GET /health`
- Readiness: `GET /ready`
- OpenAPI summary: `GET /api/v2/openapi.json`

## Admin Panel

Open:

```text
http://127.0.0.1:22417/admin
```

Log in with the value of `ADMIN_API_KEY`.

The panel can:

- Toggle anonymous public API access.
- Change anonymous and default user-key rate limits.
- Create user API keys.
- Rotate user API keys.
- Revoke user API keys.
- List key status, prefix, rate limit, and last-used time.

User API keys are shown only once when created or rotated. Store them immediately. Only the key hash is stored in MongoDB.

## Authentication

Public v2 endpoints support two modes:

- Anonymous access, controlled by the admin access policy.
- User key access via:

```http
X-API-Key: bzusr_...
```

Admin endpoints require one of:

```http
X-Admin-Api-Key: your-admin-key
```

or an admin session cookie created by logging in through `/admin`.

The admin key is not accepted as a public user API key.

## Public API v2

### List Products

```http
GET /api/v2/skyblock/bazaar/products
```

Example response:

```json
{
  "success": true,
  "data": ["WHEAT", "ENCHANTED_WHEAT"],
  "error": null,
  "pagination": null,
  "timestamp": "2026-05-28T13:34:06Z"
}
```

### Batch Latest Prices

```http
GET /api/v2/skyblock/bazaar/products/latest?ids=WHEAT,ENCHANTED_WHEAT
```

If `ids` is omitted, the endpoint returns the latest snapshot list with an internal limit.

### Single Latest Price

```http
GET /api/v2/skyblock/bazaar/products/WHEAT/latest
```

Example response:

```json
{
  "success": true,
  "data": {
    "product_id": "WHEAT",
    "buy_price": 6.4,
    "sell_price": 5.8,
    "buy_volume": 54321,
    "sell_volume": 12345,
    "buy_orders": 567,
    "sell_orders": 890,
    "timestamp": 1779975200000,
    "updated_at": "2026-05-28T13:34:06Z"
  },
  "error": null,
  "pagination": null,
  "timestamp": "2026-05-28T13:34:06Z"
}
```

### OHLCV Candles

```http
GET /api/v2/skyblock/bazaar/products/WHEAT/candles?interval=1m&range=1d&metric=mid_price
```

Supported `interval` values:

```text
15s, 1m, 5m, 15m, 1h, 1d, 1w, 1mo
```

Supported `metric` values:

```text
buy_price, sell_price, mid_price, spread
```

Optional query parameters:

| Parameter | Description |
|---|---|
| `range` | Relative window such as `1h`, `1d`, `7d`, `30d`, `1y`. |
| `start` | RFC3339/ISO 8601 start time. |
| `end` | RFC3339/ISO 8601 end time. |
| `limit` | Maximum points, clamped to `1..5000`. |

Example response item:

```json
{
  "t": "2026-05-28T13:34:00Z",
  "period_end": "2026-05-28T13:35:00Z",
  "open": 6.1,
  "high": 6.5,
  "low": 6.0,
  "close": 6.3,
  "volume": 152340,
  "samples": 4
}
```

### Line Series

```http
GET /api/v2/skyblock/bazaar/products/WHEAT/series?interval=1h&range=30d&metric=buy_price&stat=avg
```

Supported `stat` values:

```text
open, high, low, close, avg, volume
```

Example response item:

```json
{
  "t": "2026-05-28T13:00:00Z",
  "value": 6.22,
  "samples": 240
}
```

## Admin API v2

All admin endpoints require an admin session cookie or `X-Admin-Api-Key`.

| Method | Path | Description |
|---|---|---|
| `POST` | `/api/v2/admin/session` | Log in and create an admin session cookie. |
| `POST` | `/api/v2/admin/session/logout` | Log out and revoke the admin session. |
| `GET` | `/api/v2/admin/api-keys` | List user API keys. |
| `POST` | `/api/v2/admin/api-keys` | Create a user API key. |
| `PATCH` | `/api/v2/admin/api-keys/{id}` | Update key metadata, status, scopes, or limits. |
| `POST` | `/api/v2/admin/api-keys/{id}/rotate` | Rotate a key and show the new secret once. |
| `DELETE` | `/api/v2/admin/api-keys/{id}` | Revoke a key. |
| `GET` | `/api/v2/admin/access-policy` | Read public API access policy. |
| `PATCH` | `/api/v2/admin/access-policy` | Update anonymous access and default limits. |

Create a user key:

```powershell
$body = @{
  name = "Example Client"
  owner_email = "client@example.com"
  rate_limit_per_minute = 600
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

## Data Storage

MongoDB collections used by the v2 runtime:

| Collection | Purpose |
|---|---|
| `bazaar` | Raw changed snapshots. |
| `bazaar_latest` | One latest snapshot per product. |
| `bazaar_candles` | Materialized chart candles by product, interval, metric, and period. |
| `api_keys` | Hashed user API keys and per-key limits. |
| `api_settings` | Public access policy. |
| `compression_log` | Existing compression/admin statistics support. |

Retention behavior:

| Resolution | Retention |
|---|---|
| Raw 15-second snapshots | 24 hours |
| 1-minute candles | 30 days |
| 5-minute and 15-minute candles | 180 days |
| 1-hour candles | 5 years |
| Daily, weekly, monthly candles | Indefinite |

## Development Checks

```powershell
cargo fmt
cargo check
cargo test
```

## License

This project is licensed under the GNU General Public License v3.0. See [LICENSE](LICENSE) for details.
