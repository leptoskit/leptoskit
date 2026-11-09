# LeptosKit

The app you keep on a phone while you work. `leptoskit dev` prints a QR code for the guest it is serving. Scan it here, or paste the address, and the phone runs that guest. It replaces itself when you save.

The address has to be on the local network: a private IP, `localhost`, or a `.local` name, `http` or `https`, with no username or query. A release build will not run the unsigned guest.

```sh
cd examples/dev_client
cargo test
# on a machine with the Android SDK or Xcode:
leptoskit build --platform android
leptoskit build --platform ios
```

An emulator reaches the computer at `http://10.0.2.2:<port>/_leptoskit/dev`. A phone on the same Wi-Fi uses the address in the QR code.
