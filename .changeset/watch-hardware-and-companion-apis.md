---
"leptoskit_watch": minor
---

# Add watch hardware, workout, and companion sync APIs

Adds portable, `#![forbid(unsafe_code)]` watch OS and hardware APIs to `leptoskit_watch`:

- `crown`: Digital Crown / Wear OS rotary bezel `CrownConfig`, `CrownEvent`, `CrownSensitivity`, and `RotaryState` with detent tick counting.
- `aod`: Always-On Display / ambient lifecycle (`AmbientModeState`, `AmbientPolicy`) and Smart Stack / Tile `RelevanceHint`.
- `workout`: `WorkoutSession` state machine (`Prepared -> Running <-> Paused -> Ended`), `WorkoutActivity`, `WorkoutLocation`, and `WorkoutMetrics`.
- `companion`: Phone-to-watch sync (`CompanionState`, `DeliveryMode::{Immediate, ApplicationContext, BackgroundTransfer}`, `CompanionMessage`, `CompanionLink`, and `MemoryCompanionLink`).
- `interaction`: `WatchHaptic`, `WatchGesture` (`DoubleTap`, `WristFlick`, `CrownPress`, `SideButtonPress`, `ActionButtonPress`), and `WatchCapabilities`.
