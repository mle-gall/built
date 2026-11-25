# Built project format, version 1

*Spec, 2026-11-24. Implements "Built projects (Maxence, 2026-11-24)" in
[ui-decisions.md](ui-decisions.md). Replaces the fork's `.igzproj`
([project-file.md](project-file.md)), which upstream never had.*

A Built project is **one manifest file at the project root**,
`Name.builtproject`. Its models stay ordinary `.built` (or `.igz`) files in
ordinary folders, so each one can still be opened, linked or sent on its own.
The manifest holds what the people on the project share. What one person
sees (open tabs, cameras, the editable file, window layout, last scene) is
stored in a **session file** of that person's own, outside the shared folder.

The reader, writer, migration, pointer resolution and session store are
Rust (`rust/built_core/src/builtproject/`). The records/IFC registry
stays Python by design: Rust stores and merges it as an opaque JSON object,
and `core.bim_registry.validate` checks its contents.

## 1. The manifest

UTF-8 JSON, no executable or Python-specific data. Measurements remain in
metres.

```json
{
  "built_project": 1,
  "id": "ca2c12df23ac48f9a103455660d950e5",
  "files": [
    {"path": "host.built", "role": "host"},
    {"path": "parts/track.built", "role": "linked"},
    {"path": "survey.pdf", "role": "reference"}
  ],
  "links": [
    {"host": "host.built", "source": "parts/track.built",
     "placement_uid": "0123456789abcdef", "select": {"uids": ["fedcba9876543210"]}}
  ],
  "bim_registry": {},
  "ifc_identity": {
    "namespace": "ca2c12df23ac48f9a103455660d950e5",
    "project": "2gB0vM6Bf8mPFPXX3tF4sX",
    "site": "2gB0vM6Bf8mPFPXX3tF4sY",
    "building": "2gB0vM6Bf8mPFPXX3tF4sZ",
    "storey": "2gB0vM6Bf8mPFPXX3tF4sa"
  }
}
```

| Field | Rule |
|---|---|
| `built_project` | Required integer, `1`. Readers reject any other value, and reject a file that also carries `igz_format` or `built_format`. |
| `id` | Required. 32 lowercase hexadecimal characters. The project's stable identity: it names the per-user session file (§3) and never changes, including when the folder moves or the manifest is renamed. |
| `files` | Required array of `{path, role}`. `role` is `host`, `linked` or `reference`. Paths are unique. |
| `links` | Required array of `{host, source, placement_uid, select?, include_nested?}`, as in [project-file.md](project-file.md#link-selection-and-identity). |
| `bim_registry` | Required object: the whole registry of [project-registry.md](project-registry.md). |
| `ifc_identity` | Required object. `namespace` is a UUID in hexadecimal; the spatial identities are compressed IFC GUIDs. |
| anything else | Preserved on every rewrite, at any depth Built does not interpret. |

**Paths.** Every path in the manifest is relative to the directory that holds
the manifest, uses `/` as its separator on every platform, and may use `..`
(a model kept beside the project folder). Absolute paths are accepted on
read and kept as written, but Built never writes one. Moving or copying the
whole project folder keeps every relationship.

**Writing.** Every write is atomic: the new text goes to a temporary file in
the same directory, is flushed and synced, and is renamed over the manifest;
the directory is then synced. A reader therefore sees either the old manifest
or the new one. Keys are written in sorted order with two-space indentation
and a final newline, so a manifest under version control diffs cleanly.
Rust keeps every number's decimal text through its reads and writes; a
registry value Python edits is written as Python's shortest round-trip
form.

### What is shared, and why

| Content | In the manifest? | Why |
|---|---|---|
| Files and roles | Yes | The project *is* the set of models and documents that belong together. A role decides how every person's Built treats a file: a host can be the editable file, a linked source is placed by hosts, a reference is listed and never modelled. |
| Links | Yes | The link graph (which host places which source, with which subset and placement UID) is what the Project tree and reload/relink act on before any model is open. Each host still stores its own placements; the manifest is the index. Saving a host rewrites only its own edges. |
| Records/IFC registry | Yes | Classification systems, types, systems, the project's IFC metadata, export profiles, tag sets, storeys and the pset library must be one vocabulary for every model, or the records diverge. This already was the `.igzproj`'s main purpose. |
| IFC identity | Yes | Every host must export the same `IfcProject`, `IfcSite`, `IfcBuilding` and storey GUIDs, or IFC files from two models of one project cannot be federated. |
| Project Structure | Yes, inside the registry | The structure tree (`bim_registry.structure`, [project-structure.md](project-structure.md)) is already a registry section, so it is shared with the registry and needs no field of its own. A separate catalogue would duplicate it. |
| Deliveries | No | Decided in ui-decisions.md: deliveries are saved in the file, so they travel with a model that is sent on its own. A project-level delivery can be added later as an unknown-preserving field without a version change. |
| Tabs, cameras, editable file, window layout, last scene | No | Per person (§3). In the shared file they made every person's session overwrite everyone else's, which is the bug the `.igzproj`'s `worksession` field had. |
| Thumbnails, recents | No | Thumbnails live in each model; recents in the settings store. |

## 2. How a model finds its project

A model stores a pointer in `plugin_data.project`:

```json
{"path": "/absolute/at/save/Job.builtproject", "rel": "../Job.builtproject"}
```

Resolution, `resolve_pointer(host model path, pointer)`:

1. Candidates, in order: `rel` joined to the model's directory, then `path`.
2. For each candidate: when it names a `.builtproject` that exists, it is the
   project. When it names an `.igzproj`, the **sibling** with the same stem
   and the `.builtproject` suffix wins if it exists; otherwise the
   `.igzproj` itself, if it exists, is the project and is converted on first
   read (§5).
3. No candidate exists: the model has no reachable project. It uses its local
   registry fallback, and an edit to an attached registry fails rather than
   creating a new shared file silently (as today).

Opening a model never rewrites it, and neither does a conversion: an old
pointer to `X.igzproj` keeps resolving to `X.builtproject` for as long as
the model is not saved. The model's next **normal** save writes the pointer
to the resolved `.builtproject` (both `path` and `rel`), with its other
changes. The compatibility export (`.igz` for Built) keeps writing an
absolute pointer to the resolved project, as today.

## 3. The per-user session file

One file per person and project, named by the manifest's `id`:

| Platform | Directory |
|---|---|
| Any, when `BUILT_SETTINGS_DIR` is set | `$BUILT_SETTINGS_DIR/projects/` |
| macOS | `~/Library/Application Support/Built/projects/` |
| Linux and other Unix | `$XDG_STATE_HOME/built/projects/`, else `~/.local/state/built/projects/` |
| Windows | `%LOCALAPPDATA%\Built\projects\` |

The file is `<id>.json`. `BUILT_SETTINGS_DIR` is honoured first so tests and
isolated launches never touch the real Application Support folder; the Rust
API also takes an explicit directory. Linux uses the XDG *state* directory
because a session is state, not configuration. Windows uses the local, not
roaming, profile because cameras and window layout belong to a machine.

```json
{
  "built_session": 1,
  "project": "ca2c12df23ac48f9a103455660d950e5",
  "manifest": "/Users/me/Jobs/Ride/Ride.builtproject",
  "tabs": [
    {"path": "host.built", "camera": {"target": [0, 0, 0], "distance": 12}, "scene": "Entrance"},
    {"path": "parts/track.built", "camera": null, "scene": null}
  ],
  "editable": "host.built",
  "window": {"lens": 0}
}
```

| Field | Rule |
|---|---|
| `built_session` | `1`. |
| `project` | The manifest `id` it belongs to. A file whose `project` differs from its name is ignored. |
| `manifest` | Absolute path of the manifest last opened with this session. Informational: a moved or copied project folder still finds its session by `id`. |
| `tabs` | Open tabs in order. `path` is relative to the manifest directory, as in the manifest. `camera` uses the document camera fields (`views/main_window.py::_CAMERA_FIELDS`) or is `null` (keep the model's own view). `scene` is the last shown saved scene's name or `null`. |
| `editable` | `null` or one of the tab paths: the one file this person edits. |
| `window` | An object the host owns (dock lens, sizes). Preserved as written. |
| anything else | Preserved. |

Without a session file, a project opens its host files in manifest order,
with the first host editable. A converted `.igzproj` that carried a
`worksession` seeds the first session of whoever opens it (§5).

The session is written atomically whenever it changes: a tab opened,
closed or reordered, the editable file changed, a save, and window close
(cameras are taken then). Writes compare with the last written text and
skip identical ones. A session write never touches the manifest.

Two people on one project each have their own file, and so do two settings
profiles on one machine. Two windows of one person on one project share the
file; the last write wins, which only affects that person's own tabs.

## 4. Concurrency

**Today (`.igzproj`).** Each write rereads the file, replaces the section it
owns, and replaces the file atomically. A changed nanosecond mtime since the
document last read the file raises a `RuntimeWarning` and sets
`scene.project_warning`, and the last writer's whole registry wins. Undo is a
whole-registry write too, so undoing an old edit also erased a colleague's
later edits.

**Built projects: merge per entry, warn per conflict, never block.**

1. *Files and links* stay owner-scoped: saving host H replaces the edges whose
   `host` is H and adds missing file entries. Other hosts' edges and every
   other field are kept from the latest file.
2. *Registry writes are three-way merges.* A write carries the registry it was
   derived from (`base`: what this document last read) and the new one
   (`ours`). Rust rereads the file (`theirs`) and merges at two levels: each
   registry section, then each key of a section that is an object
   (`types.queue_wall`, `classification_systems.discipline`, …). An entry
   that only we changed takes ours; one only they changed keeps theirs; one
   we both changed to different values takes ours and is reported as a
   **conflict**. Undo passes the command's after-value as `base`, so it
   reverts only what that command changed.
3. *Optimistic retry.* Just before the rename, the writer checks that the
   manifest's (mtime, size) still match what it merged against. If not, it
   merges again against the newer file, up to three times. This narrows the
   race between two writers to the rename itself; it is not a lock.
4. *Reporting.* A conflict keeps ours, as today, and names the entries:
   `scene.project_warning` and the `RuntimeWarning` say "Project entries also
   changed elsewhere; yours kept: types.queue_wall". The native window shows
   it as a toast in the activity capsule. Nothing asks a question, per the
   dialog rule in ui-decisions.md.
5. *Reading.* Registry reads already refresh from disk, so another person's
   change appears at the next read without a reload prompt.

A lock file was considered and rejected: a crashed or offline Built (a
laptop asleep on a shared drive) leaves a stale lock, and clearing it needs
exactly the blocking question the dialog rule forbids.

## 5. Converting an `.igzproj`

Opening an `.igzproj`, or reading the registry of a model whose pointer
resolves to one, converts it in place:

1. Read and validate `X.igzproj` (`version: 1`).
2. If `X.builtproject` already exists, it wins: nothing is written, the old
   file is left alone, and the conversion reports "already converted". (Two
   files exist when a person on an old build wrote the `.igzproj` again;
   overwriting the newer manifest would lose data.)
3. Build the manifest: every field of the old file is kept with its value,
   except `version`, which is replaced by `built_project: 1`, and a new
   `id`. The id is the old `ifc_identity.namespace` when that is 32
   hexadecimal characters (it was already a random UUID made for this
   project, and two machines converting the same synced file then agree),
   otherwise a new random id.
4. The old `worksession` field is kept in the manifest unchanged. Built never
   reads it for anything except seeding the first session of a person who
   has none, and never writes it. Keeping it makes the conversion lossless;
   dropping it would discard whatever another person's old build relies on.
5. Write `X.builtproject` atomically, then **read it back** and compare it,
   as a JSON value, with the manifest built in step 3.
6. Only after a successful write and a successful re-read, remove
   `X.igzproj`. A failure at any step removes the partial new file if this
   conversion created it and leaves `X.igzproj` untouched.
7. Member models are never rewritten (§2).

The conversion is lossless: removing `id` and `built_project` from the new
manifest and adding `version: 1` gives back exactly the old JSON value
(tested on synthetic fixtures). The Start page's recent project row moves
from `X.igzproj` to `X.builtproject`.

**Why the old file is removed, not kept.** Keeping both means two manifests
that diverge on the first edit, with no rule for which one a model's pointer
means, and an old build that keeps writing the stale one. Section 2 makes
every model find the new file without the old one, the re-read guarantees the
new file is complete before the old one goes, and the conversion is
reversible by the round trip above. A person who still needs an `.igzproj`
for an old build can export it from the manifest
(`built_core.project_legacy_json`).

## 6. Native open, save and session

- **Open.** The native shell opens a `.builtproject` (or converts an
  `.igzproj`) itself; nothing goes to the classic process. Rust plans the
  open: the tabs to open in order, each with its role, camera and scene,
  whether it exists, and the editable file. Each existing member is opened
  through the asynchronous document open and its activity capsule; each
  reports completion or failure back to the session.
- **Missing or failing members** are a Project tree row (a file row with
  the `missing` or `failed` state) and a Problems entry, with one banner in
  the dock after opening. Opening never stops to ask.
- **Restore.** Tabs come back in the session's order, with their cameras and
  last scenes. The editable file is active.
- **Editable file.** Only the editable file accepts edits. Every other
  member is read-only: its history and agent mutations are rejected, as in
  the classic worksession. Choosing a file row's "Edit this file" makes it
  the editable one.
- **Persist on change.** Tab changes, the editable file, saves and window
  close write the session (§3).
- **Start page.** New project asks for a name and writes
  `Name.builtproject` with an empty file list and a fresh registry and IFC
  identity; Open accepts `.builtproject` and `.igzproj`.

## 7. API (Rust, exported from `built_core`)

| Function or class | Does |
|---|---|
| `project_manifest_load(path)` | Validated manifest as JSON text (envelope only; the registry is validated in Python). |
| `project_manifest_peek(path)` | A manifest or an `.igzproj`, validated, never converted. |
| `project_manifest_save(path, json)` | Atomic write of a full manifest. |
| `project_manifest_new(path, registry_json, identity_json)` | A new manifest with a fresh id; fails if the file exists. |
| `project_sync_host(path, host_rel, files_json, links_json)` | Owner-scoped files/links update (§4.1). |
| `project_write_registry(path, base_json, ours_json)` | Three-way registry merge and write; returns the conflicting entries (§4.2). |
| `project_convert(path)` | §5; returns the manifest path and what happened. |
| `project_legacy_json(path)` | The `.igzproj` value of a manifest (export and tests). |
| `project_resolve_pointer(host_path, pointer_json)` | §2; the project path or `None`. |
| `project_relative(path, dir)` | A path in manifest spelling (§1). |
| `project_members(path)` | Files and roles of a manifest or `.igzproj`, read-only (the Start page; never converts). |
| `project_session_dir(override=None)` | §3's directory. |
| `ProjectSession` | The open project in a window: plan, member states, editable file, read-only test, tree rows, Problems, session load and save. |

Python glue: `formats/project.py` (registry and filter validation around
the Rust calls), `core/project.py` (the scene's pointer, merge base and
fallback registry), `app_shell/project.py` (the native window), and the
classic `views/worksession.py`, which now reads and writes the per-user
session through `ProjectSession`. They are listed in
[python-glue.md](python-glue.md#built-projects).
