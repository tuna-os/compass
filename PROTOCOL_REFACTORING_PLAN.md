# compass-ipc Protocol Refactoring Plan

## Current State
- **protocol.rs**: 2,327 LOC
  - `Request` enum: ~900 LOC (lines 184–1093), flat with all domains mixed
  - `Response` enum: ~360 LOC (lines 1257–1616), flat
  - ~45 supporting structs/enums scattered throughout
- **Problem**: Any new domain (calculator, media, clipboard, scripts, store, windows) adds variants to the same enums, creating compile dependencies between unrelated systems

## Proposed Structure

```
crates/compass-ipc/src/
├── protocol.rs          (keep as facade/re-export hub)
├── protocol/
│   ├── mod.rs           (new: domain enum wrappers)
│   ├── clipboard.rs     (new: clipboard request/response types)
│   ├── launcher.rs      (new: launcher/search types)
│   ├── media.rs         (new: media player types)
│   ├── shortcuts.rs     (new: keyboard shortcut types)
│   ├── store.rs         (new: extension store types)
│   ├── windows.rs       (new: window manager types)
│   ├── doctor.rs        (new: diagnostics types)
│   └── power.rs         (new: power command types)
```

## Phase 1: Module Structure (This PR)
1. Create `protocol/` subdirectory
2. Extract each domain's types into dedicated module files
3. Define domain-specific `Request` and `Response` enums per domain
4. Create wrapper enums in `protocol/mod.rs` that nest domain types
5. Update `protocol.rs` to re-export via `pub use`

## Wire Format Compatibility
- **Before**: `Request::ClipboardHistory { query, limit }`
- **After**: `Request::Clipboard(ClipboardRequest::History { query, limit })`
  - Serialization: Still the same variant discriminant under the hood
  - Serde derives ensure wire format is unchanged

## Benefits
1. **Independent compilation**: clipboard, media, store modules don't compile-depend on each other
2. **Focused domains**: Each module is 100–200 LOC, not scattered in 2.3k
3. **Reduced merge conflicts**: Changes to media player types don't touch clipboard types
4. **Easier testing**: Can unit test clipboard module without instantiating store/media/windows
5. **Clearer API boundaries**: Readers see which domains exist and their request/response shapes

## Implementation Notes
- All domain modules export their own `Request` and `Response` enums
- Top-level `protocol::Request` and `protocol::Response` are wrapper enums
- No public API changes; callers still use `Request::ClipboardHistory` (or `Request::Clipboard(...)`)
- Serde still works: derive(Serialize, Deserialize) on each enum

## Risks & Mitigations
- **Risk**: Serialization changes if variant order changes
  - **Mitigation**: Rust enums preserve discriminants; Serde roundtrips correctly
- **Risk**: Consumers must know which domain to match
  - **Mitigation**: Existing callers already know this; adding modules makes it explicit

---

*Architecture Finding: compass#294*  
*Refactoring Phase 1: Establish module boundaries and re-exports*
