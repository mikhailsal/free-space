# freespace web: design system

Binding values from the design review (token sheet D15, plan
`docs/designs/free-space-disk-analyzer.md`, Pass 5) and the approved Browse
mockup B (`docs/designs/mockups/browse-b/`). CSS variables live in
`src/tokens.css`; `tests/unit/contrast.test.ts` reads that file and fails when a
text pair drops under 4.5:1.

Classifier: an app UI where the user finishes one task (find what to delete). A
dense data table with one accent. No cards, no hero, no gradients, no
marketing copy.

## Type

| Role | Face | Size / line | Weight | Used for |
|---|---|---|---|---|
| host | IBM Plex Sans | 18/24 | 500 | brand in the identity row, folder `<h1>`, inspector title |
| body | IBM Plex Sans | 16/24 | 400 | prose, empty states, full-page states |
| cell | IBM Plex Sans | 14/20 | 400 | table cells, notices, inspector rows (data exception to the 16 px rule) |
| label | IBM Plex Sans | 13/16 | 500 | form labels, chips, column headers, footer keys; nothing smaller |
| numbers | IBM Plex Mono | as the role | 400 | sizes, deltas, counts, dates, paths, commands; `tabular-nums slashed-zero` |

Fonts are bundled (D13, `fonts/`, OFL): Latin, Latin Extended and Cyrillic
subsets of Plex Sans 400/500 and Plex Mono 400, about 145 KB. `build.rs` names
each file by content hash so `serve` can cache them as immutable. No external
requests.

## Color (light only, D14)

| Token | Value | Use |
|---|---|---|
| `--bg` | `#ffffff` | page, table, inspector, footer |
| `--surface` | `#f7f7f7` | help overlay, full-page states, neutral notice |
| `--fg` | `#1a1a1a` | text |
| `--muted` | `#5c5c5c` | secondary text, inactive tabs, column headers |
| `--line` | `#e3e3e3` | borders, fill-line track |
| `--line-soft` | `#f0f0f0` | row dividers |
| `--accent` | `#0a6b4f` | selection bar, primary button, focus ring, links, active tab |
| `--accent-bg` | `#e6f3ee` | selected row |
| `--hover` | `#f5f5f5` | row hover |
| `--warn-fg` / `--warn-bg` | `#7a4b00` / `#fff3d6` | fill ≥ 80 %, age > 36 h, partial badges |
| `--danger-fg` / `--danger-bg` | `#9b1c1c` / `#ffe8e8` | incomplete scan, last error, fill ≥ 90 %, Δ growth |
| `--danger-line` | `#f3caca` | bottom rule of a red notice |
| `--ok-fg` | `#0a6b4f` | Δ shrink (the accent hue, no third green) |

`color-scheme: light`. The dark theme is a TODO; its absence is marked in
`tokens.css` with `gstack-shortcut(dec-ea8cbf40)`.

Share row tint (Browse): accent at 13 % behind the row's share of the folder,
22 % on the selected row. Muted text reaches only 4.3:1 on the selected tint,
so the selected row draws all its text in `--fg`.

## Space, size, shape

Spacing steps 4, 8, 12, 16, 24, 32 px (`--s1`…`--s6`). Rows 40 px (56 px for
two-line Top rows); identity row 32; fill line 4; folder row 52; footer 36.
Control radius 4 px; chips 999 px; table and notices square. Targets ≥ 32 px
tall on desktop, ≥ 44 px under 700 px (`--target`).

## States

| State | Look |
|---|---|
| hover | row background `--hover` |
| selected | `--accent-bg`, 3 px accent bar left, 1 px accent rule top and bottom, name 500, `aria-selected` |
| focus-visible | 2 px solid accent, offset 2 px, never removed |
| disabled | opacity .5, default cursor, `aria-disabled` |
| invalid | 1 px `--danger-fg` border, error text under the field, `aria-invalid` + `aria-describedby` |
| loading | after 300 ms, a 2 px indeterminate accent line under the header and `aria-busy` on `<main>` |

## Motion

One authored moment: the filesystem fill line grows from 0 in 400 ms
(`cubic-bezier(.2,.8,.2,1)`) on the first paint of a snapshot. Toasts fade in
over 120 ms; the progress line loops every 1.2 s. With
`prefers-reduced-motion` all three stop and the progress line is static.

## Components

Every element on the page is one of these. A new one needs a row here first.

| Component | Where | Notes |
|---|---|---|
| identity row | top | `freespace · host`, snapshot age (amber > 36 h, red > 7 d), filesystem of the current path |
| fill line | under identity | 4 px, threshold color |
| notice (D5) | notice slot | one at a time by priority, `+N notices` chip; red/amber are `role=alert`, neutral `role=status` |
| folder row | header | breadcrumbs, folder size (`≥` when a lower bound), hidden chip (D11), underline tabs, `Copy refresh command`, `?` |
| form row | Find (D9), Top (D17/D18) | labeled fields, real `<form>`, segmented Files / Folders |
| data table | `<main>` | native `<table>`, roving `tabindex`, `aria-sort`, sticky header |
| share row tint | Browse rows | see Color |
| share bar | inspector "Biggest inside" | 3 px accent at 35 % |
| chip | filters, hidden count, `+N` | label size, pill |
| inspector | `<aside aria-label="Details">` | title, size, path, notes, biggest inside, facts, CLI command, actions; a bottom sheet (`role=dialog` on its panel) under 700 px |
| action button | inspector, notices, states | primary (filled accent), outline, link |
| toast | bottom center | 6 s, `role=status`, kept in the help overlay's recent messages |
| full-page state | first run, scanning, session ended, no access (D7, D12) | surface background, one command row to copy |
| help overlay | `?` | keys, recent messages, focus trapped and restored |

## Narrow screens (D16)

v1 rule only: nothing breaks down to 360 px. The identity row wraps, Items and
Modified columns hide, the table scrolls inside its box with a sticky name
column, the inspector becomes a 40 vh bottom sheet, footer keys hide, targets
grow to 44 px. The single-column phone layout is a TODO
(`gstack-shortcut(dec-8e0d4823)` in `responsive.css`). Playwright checks 1280,
700 and 360 px with axe.

## Forbidden

- Color as the only signal: Δ carries ▲/▼, unknown sizes carry `?` and `≥`, badges carry text.
- Placeholder as label, text under 13 px, removing the focus ring.
- Inline `<style>`/`<script>` or style attributes in markup: the CSP is `default-src 'self'`.
- Fonts, scripts or images from another origin.
- Cards, shadows on content, gradients other than the share tint, dark theme bits.
