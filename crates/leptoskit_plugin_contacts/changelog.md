# Changelog

All notable changes to this project are documented in this file.

The format follows [Keep a Changelog](https://keepachangelog.com/en/1.1.0/), and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [0.3.0](https://github.com/leptoskit/leptoskit/releases/tag/v0.3.0)

### Features

#### leptoskit_plugin_contacts: permission, pick-one, and bounded query

`leptoskit_plugin_contacts` queries contacts permission, presents the system picker for one bounded contact, and optionally returns at most 32 contacts with id, display name, and optional primary phone/email. Guests enable the `guest` feature and reuse `guest::Contacts`.
