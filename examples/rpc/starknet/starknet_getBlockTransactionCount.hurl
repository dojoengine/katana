POST http://0.0.0.0:5050
Content-Type: application/json
{
    "jsonrpc": "2.0",
    "method": "starknet_getBlockTransactionCount",
    "params": [
        "latest"
    ],
    "id":1
}

HTTP 200
[Asserts]
jsonpath "$.error" not exists
jsonpath "$.result" isInteger
