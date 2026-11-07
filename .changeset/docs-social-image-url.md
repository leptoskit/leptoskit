---
leptoskit_docs: patch
---

# Social images accept absolute URLs

`Site::social_image` uses an absolute URL as it is instead of joining it onto the site's address, so a docs site can serve its social card from object storage. A relative path still joins the site URL as before. The leptoskit docs host their social card on Backblaze, which keeps the image out of the repository.
