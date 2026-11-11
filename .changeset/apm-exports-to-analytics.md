---
"release": minor
---

# Route timings go to the analytics service by themselves

When an app runs the analytics service and has route metrics turned on, its server now sends them to the service, where they're kept beside the product analytics, with no export settings to write. An export endpoint the app sets itself still wins. The service's token signs the export from the app's server and never reaches a browser.
