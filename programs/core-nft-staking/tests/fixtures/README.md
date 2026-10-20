# Test fixtures

`mpl_core.so` is the **MPL Core** program (`CoREENxT6tW1HoK8ypY1SxRMZTcVPm7R94rH4PZNhX7d`) exactly as
deployed on Solana mainnet-beta, so the tests exercise the real Core program (plugin validation,
Oracle adapters, freeze/burn rules) instead of a mock.

| | |
|---|---|
| Source | `solana program dump -u mainnet-beta CoREENxT6tW1HoK8ypY1SxRMZTcVPm7R94rH4PZNhX7d mpl_core.so` |
| Last deployed in slot | `455005931` |
| Size | `848320` bytes |
| SHA-256 | `51d41542456639a01820c747e72b9d3b4ca258930a5bbb7e78db0cda133dc209` |

To refresh it, re-run the dump command above and update the table.
