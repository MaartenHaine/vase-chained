// The dev shell's mode check (tools/dev-shell.sh, stage C), run with plain gjs before gnome-shell starts:
//
//   gjs -m DIR/mode-check.js MODE FILE UUID
//
// DIR holds ui/sessionMode.js and misc/fileUtils.js, extracted byte for byte from the installed libshell, next to
// four stubs for modules the loader imports but never reads (see dev-shell.sh). It replaces
// `gnome-shell --list-modes`, which crashes in 50.1 while the command line is parsed.
//
// gnome-shell falls back to the 'user' mode when `_modes[global.session_mode]` is missing or isn't primary after
// `_loadModes()` (sessionMode.js, SessionMode's constructor). `_loadModes()` walks gnome-shell/modes in every
// GLib.get_system_data_dirs() entry, names a mode after its file name up to the first '.json', keeps the first file
// of each name that JSON.parse accepts, skips a failed one silently, and marks every loaded mode primary. So:
//   1. FILE is in the first system data dir, and no other entry in any modes dir maps to MODE, so FILE is the only
//      file that can define it;
//   2. FILE parses the way _loadMode parses it (Gio load_contents, TextDecoder, JSON.parse, in this same SpiderMonkey)
//      and, after its property filter, has no parentMode, no components and enables only UUID;
//   3. the real listModes() then runs the real _loadModes() and prints the primary modes; dev-shell.sh requires MODE
//      among them. listModes prints exactly the modes the constructor accepts (both test `_modes[name].isPrimary`).
// Diagnostics go to stderr; stdout carries only listModes' output. Exit 1 on any failure.
import GLib from 'gi://GLib';
import Gio from 'gi://Gio';
import System from 'system';

import {listModes} from './ui/sessionMode.js';

function fail(message) {
    printerr(`mode-check: ${message}`);
    System.exit(1);
}

if (ARGV.length !== 3)
    fail('usage: mode-check.js MODE FILE UUID');
const [mode, file, uuid] = ARGV;

// _loadMode's name rule, for the uniqueness check only (the real loader decides in step 3).
function modeNameOf(name) {
    const suffix = name.indexOf('.json');
    return suffix === -1 ? name : name.slice(0, suffix);
}

// 1. FILE is the only candidate, and the first dir is its own.
const dataDirs = GLib.get_system_data_dirs();
if (dataDirs.length === 0)
    fail('no system data dirs');
const firstModes = GLib.build_filenamev([dataDirs[0], 'gnome-shell', 'modes']);
if (GLib.path_get_dirname(file) !== firstModes)
    fail(`${file} isn't in the first system data dir's modes dir ${firstModes}`);
if (GLib.path_get_basename(file) !== `${mode}.json`)
    fail(`${file} isn't named ${mode}.json`);
for (const dir of dataDirs) {
    const path = GLib.build_filenamev([dir, 'gnome-shell', 'modes']);
    let children;
    try {
        children = Gio.File.new_for_path(path).enumerate_children('standard::name',
            Gio.FileQueryInfoFlags.NONE, null);
    } catch {
        continue;
    }
    let info;
    while ((info = children.next_file(null))) {
        const name = info.get_name();
        const child = GLib.build_filenamev([path, name]);
        if (modeNameOf(name) === mode && child !== file)
            fail(`${child} also defines the ${mode} mode`);
        if (path === firstModes && child !== file)
            fail(`${firstModes} holds more than ${mode}.json: ${name}`);
    }
}

// 2. FILE parses as _loadMode parses it, and the filtered mode is the minimal one.
let parsed;
try {
    const [, contents] = Gio.File.new_for_path(file).load_contents(null);
    parsed = JSON.parse(new TextDecoder().decode(contents));
} catch (e) {
    fail(`${file} doesn't load: ${e.message}`);
}
if (parsed === null || typeof parsed !== 'object' || Array.isArray(parsed))
    fail(`${file} isn't a JSON object`);
if (parsed.parentMode !== undefined && parsed.parentMode !== null)
    fail(`${file} has a parentMode`);
if (!Array.isArray(parsed.components) || parsed.components.length !== 0)
    fail(`${file} has components`);
if (!Array.isArray(parsed.enabledExtensions) || parsed.enabledExtensions.length !== 1 ||
    parsed.enabledExtensions[0] !== uuid)
    fail(`${file} doesn't enable exactly ${uuid}`);

// 3. The real loader.
listModes();
