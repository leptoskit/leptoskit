---
"leptoskit_analytics": minor
---

Persons, groups, and sessions at the analytics service, and views the browser records on its own.

The service now builds a person for every identified id and joins a visitor's made-up id to their real one when the app identifies them, so what someone did before and after signing in is one story. Person properties keep the value of the most recent event per property, whatever order events arrived in — an offline phone's late batch no longer overwrites what you learned later. Merging two already-identified persons is refused and counted, never guessed. Each visit becomes one session row with its entry and exit pages, duration, bounce, and how it arrived: the channel from the referrer and UTM parameters, and the device, browser, and operating system parsed from the user agent at the service, so clients stay small. The browser's client now records a `$pageview` on load and on history navigation and a `$pageleave` when the page goes, with the session's first view carrying its UTM parameters and referrer; `Options::pageviews` turns that off.

Your app's server can read and manage persons — list them, read one with their events and sessions, delete one and everything stored about them, or export one as JSON — and the service's `person_profiles` setting decides who gets a person at all.

The core browser client grows from 11,985 to 13,584 bytes for the automatic views, so its size budget changes from 12 KiB to 14 KiB, decided in the open as the analytics design's size rule requires.
