---
"leptoskit_cloud": patch
---

# Hold a build's reported steps to a bound

A build worker's report is held to at most 200 steps, whatever it sent. Each step becomes an event of its deployment's and goes on its sheet, and one report could carry as many steps as its output file fits. Past the bound the last step kept says how many were left out, and a step's name is kept to 500 bytes. A report within the bound is kept as it was sent.
