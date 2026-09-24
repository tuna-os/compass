#!/usr/bin/env python3
"""Validate the published vicinae.json schema with a real JSON Schema implementation.

The schema is generated from compass_core::config by schemars, and the
`config_schema` test holds the committed copy to the types. What that test
cannot say is whether a third-party validator -- the kind an editor runs --
accepts the schema and agrees about which files are valid. This does:

  * the schema is itself valid Draft 2020-12;
  * packaging/schema/example.vicinae.json validates;
  * a config with a wrongly typed key does not (a schema that accepts
    everything would pass the first two);
  * a config with a key this build does not know still validates, because
    the Rust reader preserves unknown keys and the schema must not flag them.

Usage: scripts/packaging/check-config-schema.py   (needs `pip install jsonschema`)
"""

import json
import pathlib
import sys

import jsonschema

ROOT = pathlib.Path(__file__).resolve().parents[2]
SCHEMA = ROOT / "packaging/schema/vicinae.schema.json"
EXAMPLE = ROOT / "packaging/schema/example.vicinae.json"


def main() -> int:
    schema = json.loads(SCHEMA.read_text())
    validator_cls = jsonschema.validators.validator_for(schema)
    validator_cls.check_schema(schema)
    validator = validator_cls(schema)
    print(f"{SCHEMA.relative_to(ROOT)} is valid {validator_cls.__name__}")

    failures = []

    def expect(name, document, valid):
        errors = list(validator.iter_errors(document))
        if bool(errors) == valid:
            detail = "; ".join(e.message for e in errors) or "no errors"
            failures.append(f"{name}: expected {'valid' if valid else 'invalid'} ({detail})")
        else:
            print(f"ok   {name}")

    expect("example.vicinae.json", json.loads(EXAMPLE.read_text()), True)
    expect("empty object", {}, True)
    expect("unknown keys survive", {"launcher": {"from_the_future": 1}, "later": {}}, True)
    expect("max_results must be an integer", {"launcher": {"max_results": "fifty"}}, False)
    expect("hotkey must be a string", {"launcher": {"hotkey": 3}}, False)
    expect("installed must be strings", {"extensions": {"installed": [1]}}, False)
    expect(
        "entrypoint enabled must be a boolean",
        {"providers": {"applications": {"entrypoints": {"x": {"enabled": "yes"}}}}},
        False,
    )

    for failure in failures:
        print(f"::error::{failure}")
    return 1 if failures else 0


if __name__ == "__main__":
    sys.exit(main())
