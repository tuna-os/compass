This directory contains the following typescript projects. 
More information can be obtained on each project by navigating to their respective directories.

- `extension-manager`: a node process meant to be spawned by the Compass engine, ready to load and unload extension workers on-demand.
- `@vicinae/api`: the extension SDK. It keeps the name Vicinae publishes it under on npm, because the extensions Compass runs import it by that name (ADR-0020).
- `@vicinae/raycast-api-compat`: a private package that provides a raycast-compatible way to use the `@vicinae/api` package. It is a mere wrapper around it.

# Integration with the build system

All the projects in this directory are bundled inside a single javascript file which is then executed at startup.

Compass bundles this copy of `@vicinae/api` into its extension runtime. Extensions built against the `@vicinae/api` package on npm run unchanged.
