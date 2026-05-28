# Bazaar API v2 Usage Examples

These examples intentionally use only the v2 API surface.

## Public Requests

### List Products

```http
GET /api/v2/skyblock/bazaar/products
```

### Get Latest Data for One Product

```http
GET /api/v2/skyblock/bazaar/products/WHEAT/latest
```

### Get Latest Data for Multiple Products

```http
GET /api/v2/skyblock/bazaar/products/latest?ids=WHEAT,ENCHANTED_WHEAT
```

### Get OHLCV Candles

```http
GET /api/v2/skyblock/bazaar/products/WHEAT/candles?interval=1m&range=1d&metric=mid_price
```

### Get a Line Series

```http
GET /api/v2/skyblock/bazaar/products/WHEAT/series?interval=1h&range=30d&metric=buy_price&stat=avg
```

## Authenticated User Request

```powershell
Invoke-RestMethod `
  -Uri "http://127.0.0.1:22417/api/v2/skyblock/bazaar/products/WHEAT/latest" `
  -Headers @{ "X-API-Key" = "bzusr_..." }
```

## Admin Requests

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
