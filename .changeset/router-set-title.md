---
"leptoskit_router": minor
"leptoskit": minor
---

# Set the browser tab's title

`leptoskit_router::set_title("Plans · Admin")` names what the person is looking at. On the web it sets the document's title, so the tab and the history entry follow the screen as the person moves around without a page load. A server-rendered page still writes its first title in its own `<title>`. Native windows keep the title they were opened with for now, and on a phone it does nothing.
