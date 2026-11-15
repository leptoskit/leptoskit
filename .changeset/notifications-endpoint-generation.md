---
"leptoskit_notifications": patch
---

# Fence push endpoint ownership and credential rotation

Keep endpoint ownership until explicit revocation, enforce the live endpoint cap on reactivation, and fence delivery outcomes by credential generation so late provider answers cannot invalidate rotated credentials or change their queue. Recognize FCM unregistration only from a typed FCM error detail.
