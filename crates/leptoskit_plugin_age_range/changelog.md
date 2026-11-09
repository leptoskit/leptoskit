# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/), and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## Unreleased

### Added

#### leptoskit_plugin_age_range: declared age-range request wire

Queries whether the host can ask, then requests a band around a threshold. Responses are Declined, Unavailable, or Shared with inclusive lower/upper bounds (`upper == 0` means open-ended). Every host returns typed Unavailable until Declared Age Range / Play Age Signals wiring lands. Guests enable `guest` and reuse `guest::AgeRange`.
