# Changelog

## Unreleased

### Added

- Process-memory owned blobs: status → Available; create (≤64KiB) → id; read bounded slices (empty = EOF); release. Apple, Android, Linux, Windows, and browser Direct share the same in-memory contract; durable shared-memory / object-URL backends remain later scope.
