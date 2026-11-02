# NADI plugins

A plugin is a folder in `%APPDATA%\NADI\plugins` with a `manifest.json` and a Luau script (`main.luau`). NADI calls the script's
`handle(event)` with what the plugin may see. The script answers with what to show (its widgets, banners) and what to ask of the PC.
The script runs sandboxed. It has no files, no network and no processes of its own, and it is stopped if it runs too long or uses too much memory.

```
my-plugin/
  manifest.json
  main.luau
  test.luau      (optional: `nadi-plugin test my-plugin`)
```

Install from a GitHub repository in Settings > Plugins (its latest release's `nadi-plugins.tar.gz`, else its source). Every folder
with a `manifest.json`, up to three levels down, is a plugin.

## The manifest

```json
{
  "id": "battery",
  "name": "Battery",
  "description": "How full the battery is.",
  "version": "2.0.0",
  "kind": "luau",
  "main": "main.luau",
  "default_on": false,
  "banners": true,
  "tick": "5s",
  "budget": { "call_ms": 50, "memory_mb": 16 },
  "permissions": { "battery": true },
  "settings": {
    "low": { "label": "Low mark", "type": "range", "default": 20, "min": 5, "max": 50, "step": 5, "unit": "%" }
  },
  "actions": [{ "id": "sign_in", "label": "Sign In", "ask": "Paste the code", "then": "finish" }],
  "widgets": [{ "id": "battery", "name": "Battery", "description": "The charge", "place": "right", "rank": 30 }]
}
```

- **`kind`:** `luau`, or `face` for the frame's own looks (`look`: `eyes`, `aura`), which need no script.
- **`tick`:** calls the plugin this often with `tick` (`250ms`, `5s`, `1m`, `2h`).
- **`settings`:** each setting's `type` is `toggle`, `number`, `text`, `choice` (with `options: [{value, label}]`) or `range`. The plugin gets their values in every event.
- **`actions`:** buttons in Settings. With `ask`, a field comes with the button; what is typed goes to the `then` action as `args.text`.
- **`widgets`:** what the plugin can put on the screen.
  - `place` is where a widget goes the first time: `top`, `bottom`, `left`, `right`, `top-left`, `top-right`, `bottom-left` or `bottom-right`.
  - `rank` orders the widgets on one edge.
  - The person can move a widget, float it or hide it.
  - `many: true`: the plugin may show many of it at once, each `"<id>:<name>"` (up to 24), such as a note pinned to the screen each.
    The person does not place these: give them a panel and no peek, and ask for their window (see `window` below).

### Permissions

| Permission | The plugin is told / may do |
|---|---|
| `battery` | `battery` events |
| `system` | `system` (CPU, memory, GPU) every 2 s |
| `audio` | `audio` (level, 16 bands, beat, kick, bpm, voice, pan) |
| `viz` | its widget may show the three bars of the sound (`peek.viz`) |
| `media` | `media` (what plays) |
| `media_art` | its widget may show the cover (`"media:<source>:<art_id>"`) |
| `media_control` | `media` output: play, pause, skip, seek, rate, focus |
| `notifications` | `notifications` (other apps' toasts) |
| `processes` | `processes` (the windows, every 2 s) |
| `titles` | their titles |
| `programs` | their programs (path, name, full screen) |
| `idle` | `idle_s` (seconds since the last input) |
| `usage` | `usage` of the programs it `watch`es (CPU, memory, GPU, frames per second) |
| `apps` | `apps`: entries for the launcher's search (`{ name, open, image, badge, tip }`) |
| `dock` | `progress`: how far along a program is, on its dock icon |
| `storage` | `store` a value (sealed for this Windows user); it comes back in `start` as `store` |
| `files` | `file` when a listed file changes (its last 256 KB) |
| `net` | `fetch` (`GET`, `POST`, `PATCH`, `PUT`, `DELETE`) from the listed hosts (`"{{settings.url}}"`: the host of that setting); the answer comes as `fetched` |
| `run` | `run` a command of a listed shape (`git -C * status`); its output comes as `command` |
| `open` | `open` an address that starts with a listed prefix |

## Events

Every event has these fields:

| Field | What it is |
|---|---|
| `event` | the event's name |
| `now` | ms since 1970 |
| `tz_offset` | minutes from UTC |
| `settings` | the setting values |
| `open` | the plugin's widgets whose panels are open (`{ id: true }`) |

The events, and what each adds:

- **`start`**: `store`.
- **`tick`**: `wake` (true when it is the answer to `wake_ms`).
- **`action`**: `action`, `args`.
- **`open`**: `widget`, `open`.
- **The permissions' events:** `battery`, `system`, `audio`, `media` (`sessions`), `notifications`, `processes` (`windows`, `idle_s`), `usage`, `file` (`path`, `text`), `helper` (`data`: a line from its helper program).
- **Answers to what the plugin asked for:** `fetched` (`id`, `status`, `body`, `error`), `command` (`id`, `code`, `stdout`).

## The reply

```lua
return {
  widgets = { battery = { peek = {...}, panel = {...}, tip = "64%" } },  -- {} takes a widget away
  say = { { text = "Battery low", priority = 60, ambient = true, pill = { icon = "battery", sub = "20%", key = "battery", tone = "low", ms = 5000, buttons = {...} } } },
  cancel = { "battery" },          -- banners to take back, by key
  note = "No battery found.",      -- the line under its name in Settings (false clears it)
  wake_ms = 60000,                 -- call me (tick) in this long
  store = { ... }, fetch = { {...} }, run = { {...} }, open = "https://...", focus = { exe = "...", title = "..." },
  media = { action = "play_pause", source = "..." }, answer = {...}, reveal = "C:\\...", watch = { pid }, quiet = true,
}
```

**Idle:** `idle = true` on a widget says nothing is going on (nothing playing, nothing to show). NADI hides an idle
widget unless the person asked to see it anyway (right-click: Hide When Idle), so send one rather than taking the widget away: it
keeps its place, and its banners still come out there.

**Banners:**
- A banner comes out of the plugin's own widget: the widget becomes the banner for a while (in its place, merged or floating, as the
  person put it), and the widgets beside it make room. `widget = "id"` picks which of its widgets; without it, the first that shows.
  A plugin that says things should have a widget: one without any has its banners come out at the top.
- One with priority 90 or more asks something of the person: it comes at once.
- `ambient` ones wait while a game runs, which a plugin says with `quiet = true`.
- A plugin's banners come at most one every 3 s, and the same words not again within a minute. One with a `key` of its own may repeat.

### A widget

**`peek` (at rest):**
- What leads it is one of these:
  - `icon` (a name)
  - `image` (`"exe:<path>"`, `"media:<source>:<id>"`, `"app:<id>"`)
  - `live = { icon, tone }` (`working` sweeps)
  - `ring = { part }`
  - `vessel = { level, charging }`
  - `bars = { { part, color } }`
  - `clock = { h24, date }`
- With it, optionally: `text`, `label`, `color`, `viz`, `dot`, `button = { icon, label, action }`.

**`window`:** `{ open, remember, ground }`: `open` set brings the widget's panel out as a window (cleared, sends it back), as if
the person popped it out; it may have no peek, and then no place on an edge. `remember`: out when NADI ends, it comes out again when
it starts, on the same monitor, where it stood (a window the person popped out is not remembered). `ground`: its solid background
(`"#rrggbb"`). Its window closed, the plugin hears its widget shut (`open` with `open = false`).

**`panel` (open):**
- `title`, `sub`, `tile = { icon | image | text, color, ring, ring_color, vessel, bolt }`, `press` (the head's action), and `body`.
- `body` is a list of blocks, each `{ kind = ... }`:

| Block | Fields |
|---|---|
| `text` | `label`, `text`, `tone` |
| `facts` | `rows = { { k, v } }` |
| `stats` | `items = { { value, label } }` |
| `chips` | `items = { { icon, text, tone } }` |
| `meter` | `label`, `part`, `text`, `color` |
| `bars` | `items = { { label, part, color, text, sub } }` |
| `rings` | `items = { { label, part, color, text } }` |
| `list` | `rows = { { title, sub, progress, action, args } }` |
| `rows` | `rows = { { title, time, past } }` |
| `lines` | `rows = { { icon, text, now } }` |
| `strip` | `text`, `side`, `tone` |
| `checklist` | `title`, `items = { { state, text } }` |
| `files` | `title`, `rows = { { name, added, removed } }` |
| `graph` | `values`, `max`, `marks` |
| `spectrum` | `values` |
| `seek` | `value`, `action`, `position`, `duration`, `before`, `after` |
| `controls` | `items = { { icon, label, action } }` |
| `buttons` | `items = { { label, action, tone } }` |
| `choice` | `label`, `action`, `value`, `options` |
| `slider` | `label`, `icon`, `value`, `text`, `action`, `press` |
| `ask` | `label`, `side`, `icon`, `text`, `buttons`, `action` |
| `field` | `placeholder`, `hint`, `value`, `action` |
| `cards` | `columns` (1 to 3, 2 at first), `cards = { { color, ink, title, text, items = { { state, text, action, args } }, badge, tip, action, args, menu } }` |
| `head` | `title`, `prev`, `next`, `press` |
| `grid` | `labels`, `cells = { { t, id, state, dots } }` |
| `section` | `id`, `title`, `sub`, `tile`, `live`, `body`, `press` (opens and closes; the first is open) |

A pressed block sends `action` with `args`:
- A `seek` or `slider` sends `{ fraction }`.
- A `choice` sends `{ value }`.
- A `field` sends `{ text }`.

## The library

The `nadi` table holds:
- **`log`:** `log(...)`.
- **Time:** `duration(min)`, `time(ms, tz)`, `moment(y, m, d, h, mi, s, tz)`, `when(ms, all_day)`, `iso(s)`.
- **Data:** `json.decode(text)`, `json.encode(v)`, `ics(text, now)`.
- **Bytes:** `sha256(s)`, `random(n)`, `b64url(s)`, `urlencode(s)`.
- **`null`:** a JSON null.

## Testing

`test.luau` defines `tests = {}` with functions that call `handle` and check with `eq(a, b, what)` and `check(cond, what)`. Each test
runs against a fresh copy of the plugin. Run them with `nadi-plugin test <folder>`. Call a plugin once with
`nadi-plugin run <folder> [event.json]`.
