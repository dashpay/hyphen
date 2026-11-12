# CbTx activation fixtures

Raw testnet blocks fetched on 2026-11-12 from
`https://insight.testnet.networks.dash.org/insight-api/rawblock/<hash>`.
Each file contains the binary decoding of the response's `rawblock` hex string.

| File | Height | CbTx version | Block hash |
| --- | --- | --- | --- |
| `testnet-1577999.bin` | 1,577,999 | 3 | `0000000d4ae5bbcc2ebdf5de1502b0ac3a82931e5b890cb2ad912028f56d9f33` |
| `testnet-1578000.bin` | 1,578,000 | 4 | `0000009c0b6cbd13862a8049d0a117a4210ae6eeb90d8022cd2276e875fe639c` |

SHA-256 checksums:

```text
878efa12a187ed01adc4b1b9757cbb63f0aa68a2868c45cf7ca6aa5b6f3fa76e  testnet-1577999.bin
5afcc7818b461c96f4a8d40eeaaadd8f20deb95490f4e00d10bb3adaebd500e3  testnet-1578000.bin
```

At Hyphen revision `157c849`, both blocks decode into the storage engine's
`HyDbBlock` facts through `HyDbBlockSource::decode`. This includes the v4
coinbase's height, version and credit pool balance. These tests cover the
production decoder, not a database connect or replay of testnet history.

The pinned SDK (`2026-05-05`, `0238912b`) reads the v3 fields of the v4
payload and silently leaves its trailing 32-byte asset-unlock root unread.
Hyphen therefore omits `merkleRootAssetUnlocks` from stored payload JSON.
This confirms a missing field, but does not reproduce a block decode failure
or establish the cause of the deployed testnet stall.
