# Scene documents

One versioned JSON model is shared by the Studio, the control plane
(SQLite), and the engine. Three validators — `src/scene.rs`,
`services/control/lib/scene_validator.rb`, `apps/studio/src/scene.ts` —
run the same fixtures in `contracts/fixtures/scenes` and must agree on the
first violation's reason (`invalid-reasons.json`). JSON Schema:
`contracts/scene.schema.json`.

## Version 1 (legacy drafts)

`{version: 1, name, layers: [{id, source, x, y, width, height, visible}]}`.
Still accepted everywhere and composed exactly as before (golden tests).
Migration to version 2 is explicit (Studio "Convert to v2"), never silent.

## Version 2

```json
{"version": 2, "name": "Keynote", "canvas": {"background": "#102030"},
 "layers": [
  {"id": "slides", "type": "video", "source": "hdmi", "x": 0, "y": 0, "width": 1440, "height": 1080,
   "visible": true, "fit": "contain", "crop": {"left": 0, "top": 500, "right": 0, "bottom": 500}},
  {"id": "rear", "type": "video", "source": "phone:rear", "x": 1440, "y": 0, "width": 480, "height": 540,
   "visible": true, "fit": "cover", "opacity": 90, "rotation": -15},
  {"id": "logo", "type": "image", "asset": "0123456789abcdef", "x": 1700, "y": 960, "width": 200, "height": 100,
   "visible": true},
  {"id": "band", "type": "color", "color": "#1A2B3C", "x": 0, "y": 980, "width": 1920, "height": 100,
   "visible": true, "opacity": 60}]}
```

| Field | Layers | Rule |
|---|---|---|
| `id`, `x`, `y`, `width`, `height`, `visible` | all | as v1; the unrotated rectangle must lie inside 1920×1080 |
| `type` | all | `video`, `image` or `color` |
| `opacity` | all | integer percent 0–100, default 100 |
| `rotation` | all | integer degrees −180…180 clockwise about the layer centre, default 0; the rotated layer is clipped to the canvas |
| `fit` | video, image | `stretch` (default), `contain` (letterbox, rest transparent), `cover` (fill, crop the overflow) |
| `source` | video | `hdmi`, `phone:rear`, `phone:front` |
| `crop` | video | `{left, top, right, bottom}` in 1/10000 of the source; each pair sums to at most 9000 |
| `asset` | image | 16 hex digits; must be an uploaded asset |
| `template` | image | `{id, vars}` the image was rendered from (Studio); ignored by the engine |
| `color` | color | `#RRGGBB` |
| `canvas.background` | root | `#RRGGBB`, default black |

All numbers are integers so the three validators and the compositor agree
exactly; there is no floating point anywhere in composition.

A stored document must also fit one engine IPC frame with the
`scene.apply` envelope: the control plane refuses documents whose compact
JSON encoding exceeds 63 KiB (`document_too_large`, path `""`), checked after
every per-field rule. A request that still does not fit is refused with
413 `request_too_large` and never reaches the engine.

## Assets

PNG uploads (`POST /api/v1/assets`, raw body, `image/png`, ≤ 8 MiB,
1–4096 px per side, ≤ 256 assets) are stored content-addressed as
`STEPHANIA_ASSETS_DIR/<id>.png`, `id` = first 16 hex digits of the SHA-256.
The control plane rejects scenes that reference unknown assets and refuses
to delete an asset a scene uses. The engine loads assets only while
preparing a Program on the IPC thread, re-checks the hash, decodes to RGBA
and pre-renders static layers for the output size; a missing or modified
asset fails `scene.apply` (`invalid_scene`, `asset_unavailable`) and
Program stays unchanged. The decoded images of one Program are limited to
256 MiB of RGBA in total (four 4096×4096 images; an asset used by several
layers counts once); above that `scene.apply` fails with `invalid_scene`
(`assets_too_large`). The engine's decoded-image cache is bounded by
128 MiB as well as by entry count.

## Composition

See docs/COMPOSITOR.md. Plain video layers (stretch, no crop, no rotation,
opaque) keep the original fast path; everything else is integer-mapped
(Q8 positions, Q16 sine table) with nearest-neighbour sampling and alpha
blending. Image and colour layers are rendered once per output size.

## Transitions

`POST /api/v1/program/transition {scene_id, revision, transition?}` with
`{type: "cut"}` (default) or `{type: "fade", duration_ms: 1..5000}`. A fade
cross-fades by frame timestamps from the Program on air; the
acknowledgement names the first frame that shows any of the new Program.

## Templates (text, lower thirds)

The engine has no fonts. Text, lower thirds and HTML/CSS/SVG templates are
rendered in the Studio into a PNG (with variables such as `speaker.name`
substituted), uploaded as an asset and placed as an image layer whose
`template` records how to re-render it. Changing a variable re-renders and
re-uploads. This keeps arbitrary markup out of the media engine (§21, R05).
