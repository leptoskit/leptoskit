---
"leptoskit_cloud_app": patch
---

# Console pages can't be framed by another site

The console sent no `frame-ancestors` policy with its pages, so a site on the console's own domain could show a console page inside a frame. Tenant apps are served on the apps domain, and nothing keeps that domain off the console's site: the example configurations put both on `example.com`. A browser then sends the console's session cookie to the framed page, and one click inside the frame approves a terminal on `/device`, signs out a device on `/account`, or resets an organisation's spend cap.

Every page the console answers with now carries `Content-Security-Policy: frame-ancestors 'none'`, with `X-Content-Type-Options: nosniff` and `Referrer-Policy: no-referrer`, so a browser refuses to draw the page in a frame. The pages the control plane answers itself, which the console's own middleware never sees, get `frame-ancestors 'none'` too, added to a policy the page already sets.
