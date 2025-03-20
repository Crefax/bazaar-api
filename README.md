# Bazaar API

RESTful API service for Hypixel SkyBlock Bazaar data. This API tracks, stores, and provides access to Hypixel Bazaar data.

## Getting Started

### Requirements

- Rust (latest stable version)
- MongoDB (local installation)

### Installation

1. Clone the repository:
```
git clone https://github.com/Crefax/bazaar-api.git
cd bazaar-api
```

2. Run the application:
```
cargo run
```

The application will run on `127.0.0.1:8080` by default.

## API Reference

### Current Bazaar Data Points

#### Get the latest Bazaar data for a specific product

```
GET /api/skyblock/bazaar/{product_id}
```

This endpoint returns the most recent Bazaar data for the specified product.

##### Parameters

| Parameter   | Type   | Description                     |
|-------------|--------|---------------------------------|
| product_id  | String | Hypixel SkyBlock product ID     |

##### Response

```json
{
  "product_id": "WHEAT",
  "buy_price": 6.4,
  "sell_price": 5.8,
  "buy_volume": 54321,
  "sell_volume": 12345,
  "buy_orders": 567,
  "sell_orders": 890,
  "timestamp": 1616784000000
}
```

#### Get historical Bazaar data for a product within a specific timeframe

```
GET /api/skyblock/bazaar/{product_id}/history?hours=24
```

This endpoint returns historical Bazaar data for the specified product within a given timeframe.

##### Parameters

| Parameter   | Type    | Description                                  | Default    |
|-------------|---------|----------------------------------------------|-----------|
| product_id  | String  | Hypixel SkyBlock product ID                  | -         |
| hours       | Integer | Number of hours to look back from current time | 24        |

##### Response

```json
[
  {
    "product_id": "WHEAT",
    "buy_price": 6.4,
    "sell_price": 5.8,
    "buy_volume": 54321,
    "sell_volume": 12345,
    "buy_orders": 567,
    "sell_orders": 890,
    "timestamp": 1616784000000
  },
  {
    "product_id": "WHEAT",
    "buy_price": 6.3,
    "sell_price": 5.7,
    "buy_volume": 54000,
    "sell_volume": 12300,
    "buy_orders": 560,
    "sell_orders": 885,
    "timestamp": 1616780400000
  }
]
```

## Data Model

### BazaarData

| Field       | Type             | Description                               |
|-------------|------------------|-------------------------------------------|
| product_id  | String           | Hypixel SkyBlock product ID               |
| buy_price   | Float            | Current buy price for the product         |
| sell_price  | Float            | Current sell price for the product        |
| buy_volume  | Integer          | Total buy volume                          |
| sell_volume | Integer          | Total sell volume                         |
| buy_orders  | Integer          | Number of active buy orders               |
| sell_orders | Integer          | Number of active sell orders              |
| timestamp   | DateTime (UTC)   | Time when the data was recorded           |

## License

This project is licensed under the GNU General Public License v3.0 (GPL-3.0) - see the [LICENSE](LICENSE) file for details.

GPL-3.0 is a strong copyleft license that requires anyone who distributes your code or a derivative work to make the source available under the same terms, and also provides an express grant of patent rights from contributors to users.
