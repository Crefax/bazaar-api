
# Hypixel Bazaar API

[Bazaar tracker](https://github.com/Crefax/bazaar-tracker) records the data it receives from Skyblock API at certain intervals.

It allows you to view this data retrospectively via this API.

[https://api.crefax.net](https://api.crefax.net)
## API Usage

#### Return Product Information:

```http
  GET /api/skyblock/bazaar/${productID}?key=${api_key}
```

| Parameter | Description                |
| :-------- | :------------------------- |
| `productID` | The ID of the product. |
| `api_key` | **Required**. Your API key. |


#### Return Specific Field Information:

```http
  GET /api/skyblock/bazaar/${productID}/${field}?key=${api_key}
```
| Parameter | Description                |
| :--------  :------------------------- |
| `productID` |  The ID of the product. |
| `field` | The specific field (e.g., buyPrice). |
| `api_key` | **Required**. Your API key. |

#### Return Historical Data:

```http
  GET /api/skyblock/bazaar/${productID}/${field}/${number}?key=${api_key}
```
| Parameter | Description                |
| :-------- | :------------------------- |
| `productID` | The ID of the product. |
| `field` | The specific field (e.g., buyPrice). |
| `number` | The number of past queries to retrieve (e.g., the last 30). |
| `api_key` | **Required**. Your API key. |

  
