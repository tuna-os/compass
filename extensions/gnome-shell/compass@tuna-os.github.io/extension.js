// Compass <-> GNOME Shell helper extension.
//
// Implements the versioned contract in ./dbus (the same files as
// crates/compass-shell/dbus, which a test keeps identical): windows and the
// clipboard, the two things Mutter gives no other application a way to reach.
// Deliberately small, so a GNOME release breaks this file rather than the
// launcher. Anything not in the contract does not belong here.

import Gio from 'gi://Gio';
import GLib from 'gi://GLib';
import Meta from 'gi://Meta';
import Shell from 'gi://Shell';
import St from 'gi://St';

import {Extension} from 'resource:///org/gnome/shell/extensions/extension.js';
import * as Main from 'resource:///org/gnome/shell/ui/main.js';

const CONTRACT_VERSION = 1;
const WINDOWS_PATH = '/org/gnome/Shell/Extensions/Vicinae/Windows';
const CLIPBOARD_PATH = '/org/gnome/Shell/Extensions/Vicinae/Clipboard';

// Bursts of window events (a window opening fires several) become one signal.
const COALESCE_MS = 100;

// Preferred first. A selection's first match here is the one recorded.
const CLIPBOARD_MIME_PREFERENCE = [
    'text/uri-list',
    'image/png',
    'image/jpeg',
    'text/plain;charset=utf-8',
    'UTF8_STRING',
    'text/plain',
];

// Password managers mark secrets with this; the contract says such content
// is never emitted.
const CONCEALED_MIME = 'x-kde-passwordManagerHint';

function readInterfaceXml(extension, name) {
    const file = extension.dir.get_child('dbus').get_child(`${name}.xml`);
    const [, bytes] = file.load_contents(null);
    return new TextDecoder().decode(bytes);
}

class WindowsService {
    constructor(xml) {
        this._dbus = Gio.DBusExportedObject.wrapJSObject(xml, this);
        this._signals = [];
        this._pending = 0;
    }

    get Version() {
        return CONTRACT_VERSION;
    }

    ListWindows() {
        const windows = global.display.get_tab_list(Meta.TabList.NORMAL_ALL, null);
        return windows.map(window => {
            const workspace = window.get_workspace();
            const entry = {
                id: new GLib.Variant('u', window.get_stable_sequence()),
                title: new GLib.Variant('s', window.get_title() ?? ''),
                wm_class: new GLib.Variant('s', window.get_wm_class() ?? ''),
                wm_class_instance: new GLib.Variant('s', window.get_wm_class_instance() ?? ''),
                focused: new GLib.Variant('b', window.has_focus()),
                workspace: new GLib.Variant('i', workspace ? workspace.index() : -1),
                can_close: new GLib.Variant('b', window.can_close()),
            };
            const pid = window.get_pid();
            if (pid > 0)
                entry.pid = new GLib.Variant('u', pid);
            return entry;
        });
    }

    ActivateWindow(id) {
        const window = this._find(id);
        if (window)
            Main.activateWindow(window);
    }

    CloseWindow(id) {
        const window = this._find(id);
        if (window && window.can_close())
            window.delete(global.get_current_time());
    }

    _find(id) {
        return global.display
            .get_tab_list(Meta.TabList.NORMAL_ALL, null)
            .find(window => window.get_stable_sequence() === id) ?? null;
    }

    enable() {
        this._dbus.export(Gio.DBus.session, WINDOWS_PATH);
        const changed = () => this._changed();
        const display = global.display;
        this._signals.push([display, display.connect('window-created', (_d, window) => {
            this._watch(window);
            changed();
        })]);
        this._signals.push([display, display.connect('notify::focus-window', changed)]);
        for (const window of display.get_tab_list(Meta.TabList.NORMAL_ALL, null))
            this._watch(window);
    }

    _watch(window) {
        const changed = () => this._changed();
        this._signals.push([window, window.connect('unmanaged', changed)]);
        this._signals.push([window, window.connect('notify::title', changed)]);
        this._signals.push([window, window.connect('workspace-changed', changed)]);
    }

    _changed() {
        if (this._pending)
            return;
        this._pending = GLib.timeout_add(GLib.PRIORITY_DEFAULT, COALESCE_MS, () => {
            this._pending = 0;
            this._dbus.emit_signal('WindowsChanged', null);
            return GLib.SOURCE_REMOVE;
        });
    }

    disable() {
        for (const [object, id] of this._signals)
            object.disconnect(id);
        this._signals = [];
        if (this._pending) {
            GLib.source_remove(this._pending);
            this._pending = 0;
        }
        this._dbus.unexport();
    }
}

class ClipboardService {
    constructor(xml) {
        this._dbus = Gio.DBusExportedObject.wrapJSObject(xml, this);
        this._clipboard = St.Clipboard.get_default();
        this._selection = global.display.get_selection();
        this._ownerChanged = 0;
    }

    get Version() {
        return CONTRACT_VERSION;
    }

    GetClipboardAsync(_params, invocation) {
        this._read((bytes, mime) => {
            invocation.return_value(new GLib.Variant('(ays)', [bytes, mime]));
        });
    }

    SetClipboard(content, mimeType) {
        this._clipboard.set_content(
            St.ClipboardType.CLIPBOARD, mimeType, new GLib.Bytes(content));
    }

    // Reads the current selection in the preferred type: `(bytes, mime)`, or
    // empty bytes and an empty mime when there is nothing to offer, or when
    // the owner marked it concealed.
    _read(done) {
        const offered = this._clipboard.get_mimetypes(St.ClipboardType.CLIPBOARD);
        if (offered.includes(CONCEALED_MIME)) {
            done(new Uint8Array(), '');
            return;
        }
        const mime = CLIPBOARD_MIME_PREFERENCE.find(type => offered.includes(type));
        if (!mime) {
            done(new Uint8Array(), '');
            return;
        }
        this._clipboard.get_content(St.ClipboardType.CLIPBOARD, mime, (_clipboard, bytes) => {
            const data = bytes ? bytes.get_data() ?? new Uint8Array() : new Uint8Array();
            done(data, data.length ? mime : '');
        });
    }

    enable() {
        this._dbus.export(Gio.DBus.session, CLIPBOARD_PATH);
        this._ownerChanged = this._selection.connect('owner-changed', (_sel, type) => {
            if (type !== Meta.SelectionType.SELECTION_CLIPBOARD)
                return;
            const app = Shell.WindowTracker.get_default().focus_app;
            const source = app?.get_id() ?? '';
            this._read((bytes, mime) => {
                if (!mime)
                    return;
                this._dbus.emit_signal(
                    'ClipboardChanged', new GLib.Variant('(ayss)', [bytes, mime, source]));
            });
        });
    }

    disable() {
        if (this._ownerChanged) {
            this._selection.disconnect(this._ownerChanged);
            this._ownerChanged = 0;
        }
        this._dbus.unexport();
    }
}

export default class CompassExtension extends Extension {
    enable() {
        this._windows = new WindowsService(
            readInterfaceXml(this, 'org.gnome.Shell.Extensions.Vicinae.Windows'));
        this._clipboard = new ClipboardService(
            readInterfaceXml(this, 'org.gnome.Shell.Extensions.Vicinae.Clipboard'));
        this._windows.enable();
        this._clipboard.enable();
    }

    disable() {
        this._clipboard?.disable();
        this._windows?.disable();
        this._clipboard = null;
        this._windows = null;
    }
}
