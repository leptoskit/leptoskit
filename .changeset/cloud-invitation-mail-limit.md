---
"leptoskit_cloud": patch
---

# Invitation mail to one address is limited to five an hour

Inviting an address again mailed it again and replaced its earlier invitation, so the cap on pending invitations never grew: anyone who could invite could send one address any amount of mail from the platform's sender. An organisation may now send five invitations to one address in an hour. After that, `invite_member` answers `rate_limited` until one of that hour's invitations is an hour old, and changes nothing; other addresses aren't held up. The limit is for each organisation on its own: several organisations can still each send five.
