# Bazaar API v2 Data Lifecycle

This document describes the v2 storage model used for long-running Bazaar market tracking.

## Goals

- Keep recent market movement detailed enough for short-term charts.
- Preserve long-term market history without keeping unlimited raw snapshots.
- Serve chart data from indexed, materialized candle collections instead of expensive raw aggregations.
- Avoid storing duplicate snapshots when Hypixel returns unchanged product data.

## Tracker Behavior

The tracker polls Hypixel Bazaar data every 15 seconds.

For each product:

1. Build a snapshot from `quick_status`.
2. Compare it with the previous in-memory snapshot.
3. Skip the product if all tracked fields are unchanged.
4. Store changed snapshots in `bazaar`.
5. Upsert the current product state into `bazaar_latest`.
6. Upsert only 15-second chart candles into `bazaar_candles`.
7. Roll closed candle periods into lower-resolution candles in a separate scheduler.

Tracked fields:

- `buy_price`
- `sell_price`
- `buy_volume`
- `sell_volume`
- `buy_orders`
- `sell_orders`

## Collections

| Collection | Purpose |
|---|---|
| `bazaar` | Raw changed snapshots for recent high-resolution history. |
| `bazaar_latest` | One current snapshot per product for fast latest-price queries. |
| `bazaar_candles` | Materialized candles keyed by product, interval, metric, and period start. |
| `api_keys` | HMAC-hashed user API keys, per-key rate limits, and daily quotas. |
| `api_settings` | Public API access policy. |

## Candle Model

Candles are stored per product, interval, and period start. Each candle document contains all metric payloads, which avoids creating one document per metric.

Supported intervals:

```text
15s, 1m, 5m, 15m, 1h, 1d, 1w, 1mo
```

Supported metrics:

```text
buy_price, sell_price, mid_price, spread
```

Each metric inside a candle stores:

- `open`
- `high`
- `low`
- `close`
- `value_sum`
- `sample_count`

Each candle also stores:

- `volume`
- `buy_volume_sum`
- `sell_volume_sum`
- `period_start`
- `period_end`
- `expires_at` for retention-managed resolutions

## Retention Profile

| Data | Retention |
|---|---|
| Raw changed snapshots | 24 hours |
| 15-second candles | 24 hours |
| 1-minute candles | 30 days |
| 5-minute candles | 180 days |
| 15-minute candles | 180 days |
| 1-hour candles | 5 years |
| Daily candles | Indefinite |
| Weekly candles | Indefinite |
| Monthly candles | Indefinite |

This keeps short-term data detailed while preserving multi-year and long-term market history at lower resolution.

## Chart API Examples

Recent 1-minute candles:

```http
GET /api/v2/skyblock/bazaar/products/WHEAT/candles?interval=1m&range=1d&metric=mid_price
```

Hourly line series for 30 days:

```http
GET /api/v2/skyblock/bazaar/products/WHEAT/series?interval=1h&range=30d&metric=buy_price&stat=avg
```

Daily candles for one year:

```http
GET /api/v2/skyblock/bazaar/products/WHEAT/candles?interval=1d&range=1y&metric=mid_price
```

Monthly long-term candles:

```http
GET /api/v2/skyblock/bazaar/products/WHEAT/candles?interval=1mo&range=20y&metric=mid_price
```

## Performance Notes

- Latest price endpoints read from `bazaar_latest`, not from sorted raw history.
- Chart endpoints read from `bazaar_candles`, not from request-time aggregation over raw snapshots.
- Candle indexes are optimized for `product_id + interval + metric + period_start`.
- Public endpoints use the shared security store for response cache, rate limits, daily quotas, and policy cache.
- `REDIS_URL` enables Redis-backed shared cache/rate-limit/session behavior. Redis is required for production or multi-instance deployments.
- Without Redis, the service uses bounded in-memory fallback intended for local development only.

## Operational Notes

- `GET /ready` checks MongoDB and required security-store connectivity.
- The rollup scheduler runs once per minute and processes only closed periods.
- Retention is handled by MongoDB TTL indexes on `expires_at`.
- Daily, weekly, and monthly candles do not receive `expires_at`, so TTL does not delete them.
- User API key secrets are never stored in plaintext; only prefixes and HMAC hashes are stored.
