# `leptoskit_plugin_ble`

Bluetooth Low Energy (BLE) GATT central scanning, characteristic subscriptions, and pure-Rust standard Bluetooth SIG wearable decoders:

- Heart Rate Measurement (`0x180D` / `0x2A37`), including 8-bit/16-bit BPM, sensor contact, energy expended, and RR-interval series (1/1024 s resolution) with RMSSD calculation.
- Pulse Oximeter (`0x1822` / `0x2A5E`, `0x2A5F`) — `SpO2` % and pulse rate.
- Health Thermometer (`0x1809` / `0x2A1C`) — IEEE-11073 FLOAT temperature.
- Battery Level (`0x180F` / `0x2A19`).
- Running Speed & Cadence (`0x1814` / `0x2A53`).
- Cycling Power (`0x1818` / `0x2A63`).

Runs on iOS, watchOS, macOS (`CoreBluetooth`), Android and Wear OS (`BluetoothGatt`), and HarmonyOS / Huawei Watch (`@kit.ConnectivityKit`).
