# `leptoskit_plugin_wearable`

Direct on-watch live and passive biometric sensors for Wear OS (Pixel Watch, Samsung Galaxy Watch / One UI Watch, and other Wear OS devices), Apple Watch (watchOS), and Huawei Watch (HarmonyOS Wearable).

- **Wear OS**: Wear Health Services (`PassiveMonitoringClient`, `MeasureClient`, `ExerciseClient`), Android `SensorManager` (`TYPE_HEART_RATE`, `TYPE_STEP_COUNTER`, `TYPE_LOW_LATENCY_OFFBODY_DETECT`), and an optional runtime-checked hook for the Samsung Health Sensor SDK when present on Galaxy Watch hardware.
- **watchOS**: `HKWorkoutSession` + `HKLiveWorkoutBuilder` for live wrist metrics, `HKObserverQuery` + background delivery for passive updates, and `CMPedometer`.
- **HarmonyOS Wearable**: `@kit.SensorServiceKit` (`HEART_RATE`, `PEDOMETER`, `WEAR_DETECTION`) with `@kit.BackgroundTasksKit`.
