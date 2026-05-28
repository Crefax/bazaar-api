# Bazaar API v2 Usage Examples

These examples intentionally use only the v2 API surface. Former `/api/v1/...` and `/api/...` routes are not registered.

## Public Requests

```http
GET /api/v2/skyblock/bazaar/products
```

```http
GET /api/v2/skyblock/bazaar/products/WHEAT/latest
```

```http
GET /api/v2/skyblock/bazaar/products/latest?ids=WHEAT,ENCHANTED_WHEAT
```

```http
GET /api/v2/skyblock/bazaar/products/WHEAT/candles?interval=1m&range=1d&metric=mid_price
```

```http
GET /api/v2/skyblock/bazaar/products/WHEAT/series?interval=1h&range=30d&metric=buy_price&stat=avg
```

## Authenticated User Request

```powershell
Invoke-RestMethod `
  -Uri "http://127.0.0.1:22417/api/v2/skyblock/bazaar/products/WHEAT/latest" `
  -Headers @{ "X-API-Key" = "bzusr_..." }
```

Rate-limit headers:

```http
X-RateLimit-Limit: 600
X-RateLimit-Remaining: 599
```

Daily quota headers appear when the key has a quota:

```http
X-DailyQuota-Limit: 100000
X-DailyQuota-Remaining: 99999
X-DailyQuota-Reset: 1779993600
```

## Admin Requests with Header Auth

### Read Access Policy

```powershell
Invoke-RestMethod `
  -Uri "http://127.0.0.1:22417/api/v2/admin/access-policy" `
  -Headers @{ "X-Admin-Api-Key" = $env:ADMIN_API_KEY }
```

### Create User API Key

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

### Disable Anonymous Public Access

```powershell
$body = @{
  anonymous_public_enabled = $false
} | ConvertTo-Json

Invoke-RestMethod `
  -Method Patch `
  -Uri "http://127.0.0.1:22417/api/v2/admin/access-policy" `
  -Headers @{ "X-Admin-Api-Key" = $env:ADMIN_API_KEY } `
  -ContentType "application/json" `
  -Body $body
```

### Compression Admin Endpoints

```powershell
Invoke-RestMethod `
  -Uri "http://127.0.0.1:22417/api/v2/admin/compression/stats" `
  -Headers @{ "X-Admin-Api-Key" = $env:ADMIN_API_KEY }
```

```powershell
Invoke-RestMethod `
  -Uri "http://127.0.0.1:22417/api/v2/admin/compression/logs?limit=50" `
  -Headers @{ "X-Admin-Api-Key" = $env:ADMIN_API_KEY }
```

## Admin Requests with Session Auth

The built-in `/admin` panel handles session cookies and CSRF tokens automatically. Custom browser clients must send the `X-CSRF-Token` returned by `/api/v2/admin/session` for `POST`, `PATCH`, `PUT`, and `DELETE` calls.

## Chart Query Options

Supported intervals:

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
