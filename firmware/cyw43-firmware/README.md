# CYW43439 firmware

Firmware for the Wi-Fi chip of the Pico W, loaded at startup by the `pico_w_wifi` example.
Licensed under the [Infineon Permissive Binary License](./LICENSE-permissive-binary-license-1.0.txt).

Copied unmodified from embassy's [`cyw43-firmware`](https://github.com/embassy-rs/embassy/tree/84444a19eb57d978e3c09fcdc8e60cdd7278eb03/cyw43-firmware)
at the commit that released `cyw43` 0.7.0 and `cyw43-pio` 0.10.0.
embassy got them from [georgerobotics/cyw43-driver](https://github.com/georgerobotics/cyw43-driver/tree/main/firmware).

| File               | Contents                       | SHA-256                                                            |
| ------------------ | ------------------------------ | ------------------------------------------------------------------ |
| `43439A0.bin`      | Wi-Fi firmware                 | `5555e0261da2610a500d68c18d895cace0152bbefbf76f4aa683ebce77e3d7eb` |
| `43439A0_clm.bin`  | Country locale matrix          | `e712b3d218e8b1e2747b092e03b8b0afcb8c8c8e355d2a4a0d47b493800f3f89` |
| `nvram_rp2040.bin` | Board configuration (Pico W)   | `4904bdbb0c937bd0ac2eb2a1d62f2da4dd90e32082384e02874e8d671b0f330d` |
