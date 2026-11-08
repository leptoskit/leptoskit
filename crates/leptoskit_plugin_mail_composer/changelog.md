# Changelog

All notable changes to this project are documented in this file.

The format follows [Keep a Changelog](https://keepachangelog.com/en/1.1.0/), and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [0.3.0](https://github.com/leptoskit/leptoskit/releases/tag/v0.3.0)

### Features

#### leptoskit_plugin_mail_composer: mail composer presentation

`leptoskit_plugin_mail_composer` queries mail composer readiness and presents the system composer with to/cc/bcc/subject/body on iOS and Android. Attachments are deferred in this MVP. Completion is the OS callback only, never delivery. Guests enable the `guest` feature and reuse `guest::MailComposer`.
