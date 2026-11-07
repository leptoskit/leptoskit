# Platform capability matrix

Generated from the same host support model as `capabilities.json`; do not edit by hand. Declared build support is not runtime evidence.

| Flavour            | Platform    | Experimental | Sync Rust | Worker Rust | Async Rust | Service | Timer | Native  | Code push |
| ------------------ | ----------- | ------------ | --------- | ----------- | ---------- | ------- | ----- | ------- | --------- |
| `macos`            | `macos`     | no           | yes       | yes         | yes        | yes     | yes   | apple   | no        |
| `ios-simulator`    | `ios`       | no           | yes       | yes         | yes        | yes     | yes   | apple   | yes       |
| `ios`              | `ios`       | no           | yes       | yes         | yes        | yes     | yes   | apple   | yes       |
| `android`          | `android`   | no           | yes       | yes         | yes        | yes     | yes   | android | yes       |
| `android-x86_64`   | `android`   | no           | yes       | yes         | yes        | yes     | yes   | android | yes       |
| `harmonyos`        | `harmonyos` | yes          | yes       | no          | no         | no      | yes   | arkts   | no        |
| `harmonyos-x86_64` | `harmonyos` | yes          | yes       | no          | no         | no      | yes   | arkts   | no        |
| `wearos`           | `wearos`    | yes          | no        | no          | no         | no      | no    | —       | no        |
| `watchos`          | `watchos`   | yes          | no        | no          | no         | no      | no    | —       | no        |
| `windows`          | `windows`   | no           | no        | no          | no         | no      | yes   | —       | no        |
| `linux`            | `linux`     | no           | no        | no          | no         | no      | yes   | —       | no        |
| `tui`              | `tui`       | no           | no        | no          | no         | no      | yes   | —       | no        |
