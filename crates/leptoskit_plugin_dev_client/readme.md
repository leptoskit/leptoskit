# Development client

The LeptosKit phone app uses this plugin to follow the unsigned guest `leptoskit dev` is serving on the local network.

`guest_url` turns a scanned or pasted address into `http://<host>:<port>/_leptoskit/dev/<android|ios>/guest`. It accepts only `http` and `https`, with no username, query, or fragment, and only a host on the local network: loopback, a private or link-local address, `localhost`, or a `.local` name. Anything else is refused, so a code cannot point the phone at an arbitrary site.

On Android, `scan` opens a QR reader and returns the text. `connect` asks the running activity to follow that guest. Apple and Android hosts do this; other platforms leave the plugin out.
