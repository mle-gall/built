# Recording demo beats

What Built offers for screen recordings of the app: a keystroke overlay, a
player that replays a short scene ("beat") in a clean window exactly the
same way every time, and camera moves that glide. You record with your own
screen recorder; nothing here captures the screen.

For a steel recording, import the steel packs first: the app ships no
steel sections or connection details (they are add-ons, addons/README.md).
By hand, build them with `scripts/build_addons.py`, then in Library ›
Profiles use Import pack… for `dist/addons/steel-profiles-en.builtpack`
and `steel-connections-en.builtpack`, keeping them in My library. A beat
does it with an `addon` line (below) or `--addon`, since every beat
plays on a fresh profile.

## The keystroke overlay

View ▸ Show Keystrokes (also Settings › Display › Recording, and the
command search: "keystrokes") shows a pill at the bottom centre of the
model while you work:

- each key or chord as you press it, modifiers as ⌃ ⌥ ⇧ ⌘ (on Linux and
  Windows spelled Ctrl+, Alt+, Shift+, Super+);
- a value as you type it into the value field (`4;3`, then `↩`);
- the tool a key picked, after the key (`R  Rectangle`);
- right and middle clicks, and clicks with a modifier, as a small badge
  (`M click`, `⇧L click`); plain left clicks show nothing.

The pill keeps the last four, stays 1.2 s after the last key, then fades.
It sits above the status bar and the Paint strip and moves up over the
value field when the two would meet. Off by default; the choice is kept.

Keys typed into a text field of the panels (a name, a filter) are not
shown.

## Camera moves

Standard views (⌘1–7, the axis gizmo), Look at Face, Zoom Extents, Zoom
Selection and scene changes glide into place in a quarter of a second,
eased in and out. Settings › Navigation › Animate camera moves turns that
off (they jump). A click in the view, an orbit, pan or zoom, or Esc during
the move stops it where it is and the input takes over. Scenes with View ▸
Animation ▸ Scene Transitions on keep their own transition time.

A scripted run (`built --script`) jumps, so its frames come out the same
every time; `BUILT_CAMERA_MOTION=1` makes it glide (the beat player sets
it), `BUILT_CAMERA_MOTION=0` makes any run jump.

## Look at Face

**V** looks straight at the face under the cursor, from the side you see
it, in parallel projection until the next orbit, framed on the face. Walls
and roofs keep the world's up, so their horizontal edges run across the
screen; a floor or ceiling turns the least it can to lay its longest edge
across. Right-click a face ▸ Look at Face does the same for that face.

## Writing a beat

A beat is a text file, one step a line, `#` starts a comment. Coordinates
are logical pixels from the window's top-left (the window is `size` big;
on a 2× display the frames are twice that). Times are milliseconds.

| Step | What a hand does |
|---|---|
| `size 1600 1000` | the window's size (once, at the top; default 1600 × 1000) |
| `model examples/x.built` | start from a copy of this model (default: a blank model) |
| `addon steel-profiles-en steel-connections-en` | install these add-ons from addons/ into the fresh profile's My library before the first step (also `--addon`) |
| `keystrokes off` | do not show the keystroke overlay (default: shown) |
| `pause 500` | wait |
| `key r` / `key z shift` / `key 4 cmd` | press a key, with modifiers (`shift`, `cmd`, `alt`, `ctrl`, joined with `+`) |
| `type 4;3` | type a value one character at a time (no spaces or `#`) |
| `point 820 600 [ms]` | move the pointer there along a slightly curved, eased path |
| `click 640 520 [ms] [left\|right\|middle] [mods]` | point there, settle, click |
| `drag 100 100 300 200 [ms] [button] [mods]` | press, travel, release |
| `orbit -240 70 [ms]` | drag the middle button by this much from where the pointer is |
| `zoom 3` / `zoom -2` | mouse-wheel notches at the pointer |
| `menu look_at_face` | a menu command by id (docs/menus.md); not shown in the overlay, prefer its key |
| `capture name` | the app's own frame, only with `--shots` (tests) |
| `step state` | any raw script step (rust/built_app/src/script.rs) |

Pointer travel, settling, key holds and the gaps between typed characters
follow human timing with a little variation; the variation comes from a
fixed seed, so a beat plays identically every time (`--seed N` gives
another, still repeatable, take).

Every step goes through the window's real input path, the same one a
mouse and keyboard take: tools, inference, the value field and the
undo history behave as they do for a person. Nothing edits the model
directly, so a beat that misses a face shows the miss.

The example, [beats/box-and-cylinder.beat](beats/box-and-cylinder.beat),
draws a 4 × 3 m rectangle, pulls it up 2.5 m, draws a circle on top, pulls
it up 1.5 m, orbits and zooms to extents.

## Playing a beat

```sh
scripts/play_beat.py docs/beats/box-and-cylinder.beat
```

Built opens a window of the beat's size in front, on a fresh private
profile (your settings, recent files and logs are not read or touched),
and plays the beat. Start your recorder on that window, then let it play;
the window stays open at the end (`--quit` closes it). The beat waits
about two seconds before its first step.

- `--print` prints the compiled script steps and plays nothing.
- `--background --shots DIR` plays behind other windows, writes the
  `capture` frames to DIR and quits (what the tests do).
- `BUILT_BINARY` picks another build (default
  `rust/built_app/target/release/built`).

To find coordinates, play the beat with `--background --shots DIR` and a
`capture` step where you are unsure, and look at the frames (2× size on a
Retina display: halve the pixel positions).
