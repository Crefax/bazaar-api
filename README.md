
# Hypixel Bazaar API

[Bazaar tracker](https://github.com/Crefax/bazaar-tracker) records the data it receives from Skyblock API at certain intervals.

It allows you to view this data retrospectively via this API.

[https://api.crefax.net](https://api.crefax.net) key = anonymous

![photo](https://i.ibb.co/4WY31qV/Ekran-g-r-nt-s-2024-09-26-224532.png)

## API Usage

#### Return Product Information:

```
  GET /api/skyblock/bazaar/${productID}?key=${api_key}
```

| Parameter | Description                |
| :-------- | :------------------------- |
| `productID` | The ID of the product. |
| `api_key` | **Required**. Your API key. |


#### Return Specific Field Information:

```
  GET /api/skyblock/bazaar/${productID}/${field}?key=${api_key}
```

| Parameter | Description                |
| :-------- | :------------------------- |
| `productID` |  The ID of the product. |
| `field` | The specific field (e.g., buyPrice). |
| `api_key` | **Required**. Your API key. |

#### Return Historical Data:

```
  GET /api/skyblock/bazaar/${productID}/${field}/${number}?key=${api_key}
```
| Parameter | Description                |
| :-------- | :------------------------- |
| `productID` | The ID of the product. |
| `field` | The specific field (e.g., buyPrice). |
| `number` | The number of past queries to retrieve (e.g., the last 30). |
| `api_key` | **Required**. Your API key. |

  
