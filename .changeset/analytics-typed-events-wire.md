---
"release": minor
---

# Typed analytics events are sent by position

A typed event (`#[derive(Event)]`) now goes to the analytics service as its schema id and values, without property names, which makes it smaller on the wire. The service names the values from its event catalogue. The app's server fills that catalogue when it starts with every typed event it was built with, as the app, so no token reaches a browser or a native app.

The catalogue keeps every version of an event, so events recorded by an older build, even offline, still read correctly. When the service doesn't know an event's schema yet, it answers so and the client sends the event again with its names, so nothing is lost. An event whose values don't match its schema is refused with a reason. Untyped `capture("name", props)` works as before.
