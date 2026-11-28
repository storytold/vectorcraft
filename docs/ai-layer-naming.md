# AI-assisted layer naming

Desktop: open the Layers panel menu → **Name Layers with AI…**. Enter the name of an installed
Ollama vision model and choose the output language, whole document or highlighted rows/selection,
and group depth (1–16). Use **Play** to start or continue and **Pause** to suspend the queue after the
current group (**Continue** resumes). Ready names appear as each request completes; the counter and
progress bar show completed/total. While the first answer is awaited the bar shows an unknown extent
(the framework's moving segment, still and dashed where motion is reduced). Explicit running,
finishing, paused and complete labels distinguish the job state, so colour and motion are never the
only signal.
Review/edit them and choose **Apply Ready Names** to apply the completed portion in one undo step.
There is no Stop button. **Start Over** discards the current suggestions and checkpoint, without
undoing names already applied. Settings are retained with UI preferences.

Pausing retains ready names. Closing the dialog also keeps completed work in memory for the current
document during this app session; it does not apply any uncommitted names. Reopen the dialog and
press Play to skip completed groups. Applying a partial batch updates the checkpoint revision, so
Play can continue from the first unfinished group after applying, for example, 100 of 500 names.
Each applied batch has its own undo step. Checkpoints do not survive an app restart. Editing the
document invalidates its checkpoint; changing the naming scope or depth requires Start Over.
If a request fails, ready suggestions remain available to apply or continue.

The server defaults to `http://localhost:11434/api/chat` and the vision model to `qwen2.5vl:3b`.
The server URL, model and name language are one shared Ollama connection, so every AI dialog opens
on the same model. Plain `http://` is accepted only for a loopback host (`localhost`, `127.0.0.0/8`,
or `::1`). A remote server must use `https://`. A loopback request passes curl `--noproxy *`, so an
`http_proxy` cannot carry the preview off the machine. Ollama and an image-capable model must be installed
separately; VectorCraft does not download a model.
Desktop networking uses `curl` from PATH and reports an actionable error if it is absent. Web
builds display an unsupported message. A generation sends artwork previews to the configured
server, only after Play is clicked. A remote server receives the same previews, and only over
HTTPS. There is no API-key setting.

Empty containers are skipped and keep their existing names.

Each request contains one 512-pixel preview of one target group, fitted on white, and its numeric ID. This avoids small vision models
assigning button text from the overview or another contact-sheet tile to an unrelated group.
Existing names stay in the review UI and are omitted from the model prompt to prevent copying
technical placeholders. The dialog shows processed/total groups while generation runs.

The implementation follows the [Ollama chat API](https://docs.ollama.com/api/chat): base64 PNG,
`stream: false`, a strict JSON schema in `format`, response in `message.content`. The schema requires
a `names` array with exactly one bounded name and the exact target ID. The prompt repeats the schema,
temperature is zero and the context window is 8192 tokens. Ordinary JSON mode alone cannot guarantee
this structure; see [structured outputs](https://docs.ollama.com/capabilities/structured-outputs).

Output is untrusted. The app requires exactly one bounded, nonblank name for every requested ID;
malformed JSON, extra/duplicate IDs, missing names and control characters are rejected. Requests time
out after 120 seconds (10 seconds to connect), responses are limited to 1 MiB, and candidate lists
to 1000 groups. Narrow the scope or depth for larger documents. Closing the dialog or choosing Start Over stops further requests; an in-flight request may finish
before its timeout, and its later result is discarded. Pausing lets that request finish and keeps
its result, then sends no further requests until Play.

Applying names checks the document UID and revision captured before generation. If the document
changed, generate again. It modifies only the internal layer/group names: tree order, node IDs,
geometry, paint, clips and resources stay intact. The existing SVG exporter handles names and XML
escaping; this feature does not rewrite arbitrary SVG source in place or convert groups into layers.

Automation:

- `ui.nameLayers {}` opens the settings/review dialog (dialog `layerNames`). Its fields `endpoint`,
  `model`, `language`, `scope` and `depth` can be set while no run is going; `names` edits ready names
  by id; `action` set to `play`, `pause` or `startOver` presses that button. OK is Apply Ready Names.
- `layer.namingCandidates {scope: "all" | "selection", depth: 1..16}` returns `{uid, revision, rows}`.
- `layer.applyNames {uid, revision, names: [{id, name}]}` applies a reviewed mapping in one history step.

Validation covers read-only review/cancellation, atomic rejection, stale revisions, Undo/Redo,
selection/depth, unchanged rendered pixels, SVG name round trips, malformed model output, a local
mock Ollama HTTP exchange including real rendered previews, the dialog driven through the window's
dialog host (open, pump, Apply, Cancel and close/reopen), and automation fields. A real
model is required separately to assess semantic naming quality; protocol tests do not measure it.

The dialog was moved onto the craft UI framework on 2026-11-28 (Form dialog, Select, Number and
Text fields, the framework's Progress bar); the worker, request format and validation are unchanged.

The opt-in `real_ollama_returns_the_required_names_array` test checks a running local model on four
synthetic groups. It is a limited live compatibility check, not a semantic accuracy benchmark:

```sh
VECTORCRAFT_NAMING_OLLAMA_MODEL=qwen2.5vl:3b cargo test -p vectorcraft-ui-egui real_ollama_returns_the_required_names_array --lib -- --ignored --nocapture
```
