## Run server

`cargo run`

## Get server's public key

`GET http://0.0.0.0:3000/info`

## Get random numbers

```js
const response = await fetch("http://0.0.0.0:3000/stark_vrf", {
  method: "POST",
  headers: {
    "Content-Type": "application/json",
  },
  body: JSON.stringify({ seed: ["0x5733e5c2c8030bc06888747525b1a1f0242ca770c9387b58a4529df0ca55499"] }),
});
const json = await response.json();
```

which will return:

```js
{
    "result": {
        "gamma_x": "0x6ac32f9b3c0bef88e4e1ba77d2f77fa603bbd4b42cca6405c5bdf7a16821d75",
        "gamma_y": "0x50e3875635d84cb0038dbccb309b1134e101aee38c02001e456d5988b39fd43",
        "c": "0x111fd321fb9b48c651a871fcfeea71a0f521ba04fe54f81caf2f407cba0979f",
        "s": "0x437d280a8eb8e1e57c291af359738118c5b64b00ff5bd209d86ef83b051a2c6",
        "sqrt_ratio": "0xa99242633a4f2a7b31c4c7d3bee93dad3bd946a68cfac753dbd4bbd837d8c0",
        "rnd": "0x41c6b570b6720f205da6ef692021fe3625bbbab1ef5ea0ecea470e2d93b7982"
    }
}
```

## Verify proof in Cairo

See https://github.com/dojoengine/stark-vrf
