---
"leptoskit_cloud": patch
---

# Runtime logs hide the environment's own secrets

A cluster's worker reports an environment's runtime logs to the control plane, which kept each line as the worker sent it. A process that printed one of the environment's secret values left it readable in the logs by anyone who could view the project. The control plane now opens the environment's current secrets as a report arrives and hides them from its lines and from a failed process's reason, from eight bytes up. A value shorter than that, or one the environment replaced or removed after the process started, is still kept as printed. A report is refused while a secret can't be opened, rather than kept as printed.
