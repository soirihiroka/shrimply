Agent Editing CLI
=================

The Shrimply binary creates, inspects, edits, and renders projects directly.
It supports native GTK, Qt, and AppKit editors and headless project sessions.
There is no separate agent server executable or client registration step.
Use the installed ``shrimply`` binary (``shrimply.exe`` on Windows),
``target/debug/shrimply``, or ``target/qt/debug/shrimply`` for Qt on Linux.

Install the terminal command
----------------------------

Open :menuselection:`Preferences --> External` (on macOS,
:menuselection:`Settings --> Integrations`). The :guilabel:`CLI` row has an
:guilabel:`Install` button, or :guilabel:`Overwrite` when a command already exists.
The action selects the copy of Shrimply whose settings you are using.
Open a new terminal after installation, then run ``shrimply --help``.

On Linux and macOS, installation creates a launcher in ``~/.local/bin`` and
adds that directory to the startup configuration for Bash, Zsh, and Fish when
available. Existing shell configuration is preserved. Overwrite replaces the
user launcher; it does not modify another installation's executable.

Flatpak installs the launcher on the host through ``flatpak-spawn --host``.
It uses ``flatpak run`` with the running app's installation, architecture, and
branch, preserving command arguments, exit status, and the working directory.
The launcher continues to work after Flatpak updates.

On Windows, installation adds the application's directory to the user PATH,
keeping the executable beside its bundled libraries and editor. Overwrite
moves that directory ahead of other user PATH entries. If a different system
command takes precedence, installation reports the conflict instead of
changing system-wide files or PATH settings.

Discover an action
------------------

Start with a task description. Search runs locally over the command catalog,
argument descriptions, and common editing synonyms, returning up to five
compact JSON matches. It does not load a model or contact a service.

.. code-block:: console

   $ shrimply cli search "replace caption text"
   $ shrimply text replace --help
   $ shrimply schema text replace

Commands use consistent resource/action paths and named options. Use the
schema to inspect nested objects, enums, defaults, and required fields.
Output is JSON, indented on a terminal and compact when piped. Progress and
logging go to stderr. Exit status is 0 for success, 1 for execution failure,
and 2 for invalid arguments. Errors provide a code, message, and recovery hint.

.. code-block:: console

   $ shrimply project create --project /tmp/movie.shrimp --name Movie --fps 30000/1001
   $ shrimply clips list --project /tmp/movie.shrimp --all --recursive
   $ shrimply clips move --project /tmp/movie.shrimp --item-id UUID --start-frame 120

Use explicit project paths on every project command; relative paths work.
Times are zero-based integer frames. Frame ranges are half-open. Frame rates
use exact fractions; CLI ``--fps`` also accepts integer and decimal input.
Normal project launching and ``shrimply render PROJECT -o OUTPUT`` remain
available.

Arguments and addresses
-----------------------

Use named flags for simple values. For nested fields, pass JSON or ``@file``.
Boolean options can be bare for true; use ``--enabled=false`` (with ``=``)
for an explicit value. ``--body`` accepts complete operation arguments as inline JSON, ``@file``,
or ``@-`` for stdin. It cannot be mixed with individual operation flags.

.. code-block:: console

   $ shrimply clips move --project movie.shrimp --address @address.json --start-frame 120
   $ shrimply edit apply --project movie.shrimp --body @edits.json

Clip addresses contain ``kind``, ``sequence_path``, ``track_id``, and
``item_id``. Track addresses omit ``item_id``. Item IDs are a shortcut only
when they resolve to a single presentation. Shared folded sequences can
have several presentations; ambiguous IDs return candidate addresses.
Scopes default to root consistently. Use an explicit ``scope`` object to
edit or query another sequence presentation. Track queries include empty tracks.
``clips list`` supports filtering and pagination; ``--all`` removes the
pagination limit and conflicts with ``--offset`` and ``--limit``.

Atomic batches use the typed ``operations`` schema. Each operation has
``type`` and ``args`` fields. Operations are applied in order to a clone,
validated, and committed as one undoable edit. For example:

.. code-block:: json

   {
     "frame": 120,
     "operations": [
       {
         "type": "move_clip",
         "args": {
           "address": {"kind": "video", "sequence_path": [], "track_id": "TRACK_UUID", "item_id": "ITEM_UUID"},
           "offset_frames": 24,
           "collision": "reject"
         }
       }
     ]
   }

Replacing an animation source
-----------------------------

Use ``clips replace-source`` to change the source of an existing video clip
without deleting it. The clip keeps its UUID, timeline bounds, source offsets,
grouping, transforms, effects, compositing and transitions. Source-specific
settings reset to the new source, and source dimensions/duration are inspected
again. Audio tracks are not added or changed.

.. code-block:: console

   $ shrimply clips replace-source --project movie.shrimp --item-id UUID --source scene.sanim --link --dry-run
   $ shrimply clips replace-source --project movie.shrimp --item-id UUID --source scene.sanim --link --if-revision REVISION

The default copies the source into project media; ``--copy-root DIRECTORY``
includes sibling imports, data and fonts. ``--link`` retains the external source
and conflicts with ``--copy-root``. The source must have one video/visual stream;
audio-only and multi-stream replacements fail explicitly. OBJ files require
new scene modifiers and should be inserted as new clips.

The corresponding atomic edit operation is ``replace_clip_source`` with
``address``, ``source``, optional ``link`` and optional ``copy_root`` arguments.
Dry-run validates the target/path and defers source inspection and execution.

Preview before editing
----------------------

.. code-block:: console

   $ shrimply clips move --project movie.shrimp --item-id UUID --start-frame 120 --dry-run
   $ shrimply edit apply --project movie.shrimp --body @edits.json --dry-run

Dry-run prepares ordinary edits in memory and returns before/after changes,
validation status, and a base revision. It does not change the project,
undo history, playhead, selection, persistent caches, or project media. It
does not start a session or execute source files, conversions, generation,
or service calls. Those operations report planned effects and deferred
checks instead. Later batch operations depending on deferred work are also
deferred. Generated IDs in previews are provisional.

Use ``--if-revision REVISION`` from an inspection or preview response to
reject an edit if project content changed. Revisions exclude view state.
A preview is not a reservation; execution revalidates current state and sources.

Replace stored text
-------------------

.. code-block:: console

   $ shrimply text replace --project movie.shrimp --track-id UUID --find old --replace new --global --dry-run
   $ shrimply text replace --project movie.shrimp --all --expression 's|hello (world)|Hello \1|gi' --dry-run

Select addresses, a track, a frame range, or explicitly ``--all``. Scope and
recursive selection are supported. Replacement operates on stored caption
and comment strings and all native text clip constants/keyframes. Caption
markup is part of the stored string. Expression source code, styles, IDs,
and timing are preserved. Shared logical items are edited once.

The default replaces the first match per stored string. ``--global`` or
``g`` replaces every match; ``--ignore-case`` or ``i`` ignores case.
Literal replacement uses ``--find`` and ``--replace``. Substitution uses
``s<delimiter>pattern<delimiter>replacement<delimiter>gi``, with a punctuation
delimiter and Rust regex syntax. Replacement supports ``&`` for the match,
``\0`` through ``\9`` for captures, and escaped ampersands, backslashes,
and delimiters. Other sed commands and flags are rejected. Zero matches is
a successful no-op. Results include per-item changes and replacement counts.

Sessions, history, and media
----------------------------

An open editor owns its live project state. Commands attach to that session
and update its timeline, inspector, preview, and undo history. When no owner
exists, a mutating command starts a managed headless instance of the same
binary. Read-only commands can inspect the saved file without a session.
An unreachable owner is an error; the CLI never edits a second copy behind it.

There is no persistent agent connection or connect/disconnect step. Each CLI
invocation uses short-lived connections and exits after its response. The
project owner remains open. ``project open`` reuses an existing owner, even
with ``--headless``; it does not switch an open GUI into headless mode.
Responses identify their state with ``execution_mode``: ``live`` for a GUI,
``headless`` for a managed owner, or ``saved`` for a file read without an owner.

Multiple CLI processes can target the same project. The owner serializes
requests, so concurrent edits do not overwrite independent changes. Reads
observe a complete state before or after each edit. For decisions based on a
previous read, pass its ``content_revision`` as ``--if-revision``; an intervening
content edit causes a revision conflict. Without that guard, commands execute
against the state when they reach the owner, in arrival order.

The GTK title icon indicates current or recent editing API activity, including
reads. It remains visible for three seconds after the final client disconnects.
GTK and AppKit poll the editing API independently of display callbacks, so
requests continue processing while their windows are minimized or off-display.

.. code-block:: console

   $ shrimply project open --project movie.shrimp --headless
   $ shrimply history undo --project movie.shrimp
   $ shrimply history redo --project movie.shrimp
   $ shrimply project save --project movie.shrimp
   $ shrimply project close --project movie.shrimp

Headless sessions preserve undo/redo and playhead state until explicitly
closed. ``project serve`` runs one in the foreground. Close a headless owner
before opening the project in a GUI. ``project close`` never closes GUI
windows. API edits are persisted before success is reported. Undo keeps
imported media so redo remains valid. Undo history does not survive a
session restart.

``project close`` succeeds only after the headless owner releases the project,
so the next command can safely open it again. Requests queued after close has
been accepted fail rather than editing a shutting-down session. Inspect state
before retrying a failed edit; transport failures do not prove it was rolled back.

For stored speech scripts, use ``clips list --source-kind tts --all --recursive``
and ``clips get --item-id UUID``. The script is at
``result.metadata.source.inputs.text.value``. It describes the full generated
source, including speech removed by trimming or splitting. Inspect ``time_offset``
and the clip bounds separately; stored text is not a transcript of the audible range.

Imports copy into project media by default. ``link=true`` retains external
paths. Existing track, collision, and folded-sequence rules are preserved.
Executable sources retain Shrimply's trust requirements; agent operations
do not grant trust implicitly. Speech operations use the configured compute
server. Missing hardware, services, or platform capabilities produce errors.

``frames render`` returns PNG data; ``--output PATH`` writes a new file and
returns its path. Existing output files are not replaced. Artwork is omitted
from clip inspection unless ``--include-artwork`` is requested.

Migration from the former integration
--------------------------------------

All former tools and resource queries are available through named commands:

.. list-table::
   :header-rows: 1

   * - Former operation
     - Binary command
   * - create_project / connect_project
     - project create / project open
   * - get_editor_state / get_version / list_scopes
     - editor get / version / scopes list
   * - query_clips / get_clip / get_clip_info
     - clips list / clips get / clips inspect
   * - get_track / create_track / delete_track
     - tracks get / tracks create / tracks delete
   * - insert_files / insert_captions / insert_text / insert_tts
     - clips insert-files / insert-captions / insert-text / insert-tts
   * - replace_clip_source
     - clips replace-source
   * - move_clip / trim_clip / delete_clips / set_clip_properties
     - clips move / trim / delete / set
   * - set_video_transform / set_clip_transitions
     - clips transform / clips set-transitions
   * - upsert_keyframes / delete_keyframes
     - keyframes upsert / keyframes delete
   * - query_expressions / set_expression
     - expressions list / expressions set
   * - upsert_property_expression / delete_property_expression
     - property-expressions upsert / property-expressions delete
   * - set_track_enabled / set_caption_track_language
     - tracks set-enabled / tracks set-language
   * - get_manim_clip / set_manim_clip / reload_manim_source
     - manim get / manim set / manim reload
   * - list_tts_models / generate_tts
     - tts models list / tts generate
   * - list_stt_models / transcribe_audio
     - stt models list / stt transcribe
   * - seek_playhead / view_frame / analyze_transparent_fill
     - player seek / frames render / transparent-fill analyze
   * - run_edit_script
     - edit apply
   * - Editor-state / all-clips / item resource queries
     - editor get / clips list --all --recursive / clips get --item-id
   * - Editing API documentation resource
     - --help / cli search / schema

Additional commands expose project settings/save, history undo/redo, clip
split/duplicate, track reordering, and bulk text replacement.
