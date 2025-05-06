window.BENCHMARK_DATA = {
  "lastUpdate": 1746547388151,
  "repoUrl": "https://github.com/dojoengine/katana",
  "entries": {
    "Benchmark": [
      {
        "commit": {
          "author": {
            "email": "evergreenkary@gmail.com",
            "name": "Ammar Arif",
            "username": "kariy"
          },
          "committer": {
            "email": "noreply@github.com",
            "name": "GitHub",
            "username": "web-flow"
          },
          "distinct": true,
          "id": "92e5fe8f186b6af9f2fb03739a5a66bc22eed6c8",
          "message": "ci(test): explorer on reverse proxy (#58)",
          "timestamp": "2025-05-02T22:08:37+08:00",
          "tree_id": "e543fbafb71425486a9190af6369031a88ccde5f",
          "url": "https://github.com/dojoengine/katana/commit/92e5fe8f186b6af9f2fb03739a5a66bc22eed6c8"
        },
        "date": 1746196099496,
        "tool": "cargo",
        "benches": [
          {
            "name": "decompress world contract",
            "value": 2990472,
            "range": "± 21925",
            "unit": "ns/iter"
          },
          {
            "name": "Concurrent.Simulate/Blockifier.1",
            "value": 391889,
            "range": "± 6631",
            "unit": "ns/iter"
          },
          {
            "name": "Concurrent.Simulate/Blockifier.1000",
            "value": 2858955025,
            "range": "± 253445061",
            "unit": "ns/iter"
          },
          {
            "name": "Invoke.ERC20.transfer/Blockifier.Cold",
            "value": 16810688,
            "range": "± 454656",
            "unit": "ns/iter"
          },
          {
            "name": "Katana.Startup",
            "value": 127900822,
            "range": "± 1191613",
            "unit": "ns/iter"
          }
        ]
      },
      {
        "commit": {
          "author": {
            "email": "evergreenkary@gmail.com",
            "name": "Ammar Arif",
            "username": "kariy"
          },
          "committer": {
            "email": "noreply@github.com",
            "name": "GitHub",
            "username": "web-flow"
          },
          "distinct": true,
          "id": "ea95bd5d9cf5f960d6818b3d2233106e48b9a25d",
          "message": "fix(rpc): wrongly evaluating skip fee flag (#65)",
          "timestamp": "2025-05-06T23:43:35+08:00",
          "tree_id": "338eae02ff1adc686d10e6275288f4b410cbcb42",
          "url": "https://github.com/dojoengine/katana/commit/ea95bd5d9cf5f960d6818b3d2233106e48b9a25d"
        },
        "date": 1746547386736,
        "tool": "cargo",
        "benches": [
          {
            "name": "decompress world contract",
            "value": 2988667,
            "range": "± 18123",
            "unit": "ns/iter"
          },
          {
            "name": "Concurrent.Simulate/Blockifier.1",
            "value": 387126,
            "range": "± 3398",
            "unit": "ns/iter"
          },
          {
            "name": "Concurrent.Simulate/Blockifier.1000",
            "value": 2774541931,
            "range": "± 274774857",
            "unit": "ns/iter"
          },
          {
            "name": "Invoke.ERC20.transfer/Blockifier.Cold",
            "value": 16175297,
            "range": "± 130687",
            "unit": "ns/iter"
          },
          {
            "name": "Katana.Startup",
            "value": 126269137,
            "range": "± 1562887",
            "unit": "ns/iter"
          }
        ]
      }
    ]
  }
}