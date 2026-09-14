# ADR-0012: Compass is the public brand; legacy identifiers migrate at cutover

**Status:** Accepted · **Date:** 2026-09-14 · Supersedes: ADR-0007 decision 2

## Context

ADR-0007 kept Vicinae as both the public brand and every compatibility identifier. The fork now has
its own product direction and the project owner has chosen Compass as the public brand. Continuing
to present the upstream name in the README, application launcher and package metadata makes the
fork's ownership and Rust migration needlessly ambiguous.

Changing every identifier at once would also break existing config, shell aliases, extension
manifests, deep links, DBus permissions and the side-by-side engine comparison that the migration
uses as a safety mechanism.

## Decision

**Compass is the public project and application name now.** The README, distributable metadata,
desktop display name and new visual identity use Compass.

The following remain compatibility identifiers until the Phase 7 cutover supplies aliases and a
tested migration: the `vicinae` executable and package, `com.vicinae.Vicinae` Flatpak/application
ID, socket and config paths, DBus interfaces, URL schemes and `@vicinae/api` extension package.
User-facing copy should not call the product Vicinae merely because one of those identifiers is
visible in a command.

## Consequences

- Existing users can test the Rust engine without moving their data or changing automation.
- Packaging can present Compass while retaining an application ID compatible with installed grants.
- Phase 7 must define aliases, data migration and rollback before technical identifiers change.
- Upstream Vicinae remains credited as the source project and behavioural reference.
