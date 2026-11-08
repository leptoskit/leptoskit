# Meta Quest / Oculus host (experimental)

This folder is the leptoskit shell stub for **Meta Quest**. The Wasm guest and shared render-transaction ABI are ready in principle; OpenXR packaging and device install are not.

## Status

| Piece                            | State                                                 |
| -------------------------------- | ----------------------------------------------------- |
| Host stubs under `hosts/oculus/` | Written by `leptoskit native prepare oculus`          |
| Capability manifest              | `leptoskit_platform_oculus`                           |
| APK / Quest store packaging      | Not implemented                                       |
| Code push                        | Not enabled                                           |
| Runtime plugins                  | Declared for a future OpenXR host; not registered yet |

## Package

- Application id: `__PACKAGE_ID__`
- Display name: `__APP_NAME__`

## Next steps for platform authors

1. Wire an OpenXR / Meta Spatial activity that loads the Wasm guest through `leptoskit_bridge_ffi` (or a Quest-specific NDK bridge).
2. Apply `json/1` or `LKB1` render transactions to a panel (phase 1) or world-space quads (phase 2).
3. Map controller / hand rays to `NativeEvent` presses and scrolls.
4. Only then enable packaging in the CLI platform provider.

See the leptoskit platform-extensions design for the long-term out-of-tree sidecar path.
