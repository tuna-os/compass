# The Compass GNOME Shell extension

GNOME gives applications no way to list other windows or watch the clipboard:
Mutter implements no foreign-toplevel or data-control protocol, and
`org.gnome.Shell.Introspect` is allowlisted to the portal backends. On GNOME
50 and 51 this extension is therefore the only way Compass can switch windows
or keep clipboard history. App search and launching do not need it.

It implements exactly the versioned contract in
[`crates/compass-shell/dbus`](../../crates/compass-shell/dbus) and nothing
else. `compass-shell`'s `extension_contract` test keeps the two in step: the
XML copies here must equal the contract files, the version must match, and
every method, signal and window field must be implemented.

## Install it by hand

```sh
cd extensions/gnome-shell
gnome-extensions pack compass@tuna-os.github.io --extra-source=dbus --force
gnome-extensions install --force compass@tuna-os.github.io.shell-extension.zip
```

Then log out and back in (a Wayland session cannot reload Shell in place) and
enable it:

```sh
gnome-extensions enable compass@tuna-os.github.io
vicinae doctor   # gnome.shell-extension should now report contract v1
```

ADR-0004 decides how users get it without this: extensions.gnome.org, and
baked into the Bluefin image.

## What it does, and deliberately does not

- **Windows**: lists windows most-recently-used first (`Meta.TabList`), focuses
  and closes them, and emits `WindowsChanged` coalesced over 100 ms.
- **Clipboard**: reads and writes the clipboard, and emits `ClipboardChanged`
  on every clipboard owner change. Content a password manager marks with
  `x-kde-passwordManagerHint` is never emitted.
- It keeps no history and no state. Compass's engine does that.
