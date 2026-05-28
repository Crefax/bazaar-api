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
6. Upsert chart candles into `bazaar_candles`.

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
| `api_keys` | Hashed user API keys and per-key rate limit settings. |
| `api_settings` | Public API access policy. |

## Candle Model

Candles are stored per product, interval, and metric.

Supported intervals:

```text
15s, 1m, 5m, 15m, 1h, 1d, 1w, 1mo
```

Supported metrics:

```text
buy_price, sell_price, mid_price, spread
```

Each candle stores:

- `open`
- `high`
- `low`
- `close`
- `volume`
- `sample_count`
- `period_start`
- `period_end`

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
- Public endpoints use in-process cache and rate-limit fallback.
- `REDIS_URL` is reserved for optional Redis wiring; when Redis is not wired, the service remains functional with local fallback.

## Operational Notes

- `GET /ready` checks MongoDB connectivity.
- The retention scheduler runs hourly.
- Daily, weekly, and monthly candles are not deleted by the retention scheduler.
- User API key secrets are never stored in plaintext; only hashes and prefixes are stored.
