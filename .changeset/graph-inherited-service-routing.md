---
"release": patch
---

# Reject inherited routes that graph releases cannot deliver

Graph releases now reject alongside-app `public` and `routes` settings inherited from a service package's metadata. Use the service's explicit graph domain and mount instead. Deployment no longer silently omits those inherited routing requirements.
