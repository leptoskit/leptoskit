# `leptoskit_plugin_watch_sync`

Bidirectional messaging and durable background transfers between a watch and its paired phone:

- **Apple (iOS ↔ watchOS)**: `WatchConnectivity` (`WCSession.sendMessage`, `transferUserInfo`).
- **Android ↔ Wear OS (including Samsung Galaxy Watch)**: Google Play Services Wearable (`MessageClient`, `DataClient`, `NodeClient`).
- **Huawei HarmonyOS ↔ Huawei Watch**: `@kit.WearEngine` P2P messaging when available.
