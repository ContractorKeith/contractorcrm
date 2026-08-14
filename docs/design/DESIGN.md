# ContractorCRM design guide

Status: v0.1 baseline
Updated: 2026-08-14
Suite sibling: `ContractorProject` — see its `DESIGN.md`
Companion: `ContractorCRM Logo.dc.html`

ContractorCRM is the second module in the suite. It shares the **Industry** foundation with ContractorProject without exception: same ground, same steel accent, same Barlow Condensed over Barlow, same square corners, hairline borders, and blueprint registration marks. Only the mark and the product-specific surfaces differ.

The rule for the suite: **foundations are copied, never re-decided.** If a token needs to change, it changes in every module at once. Anything a module invents on its own must be provably local to that module's domain.

---

## 1. Principles

These are the suite principles, identical across modules.

**1. The data is the interface.** Chrome recedes; type, rules, and alignment carry the hierarchy. No decorative panels, no gradients, no illustration inside working views.

**2. Drawn, not filled.** Cards and panels are line drawings — square-cornered, hairline-bordered, transparent. Fills are reserved for meaning, never decoration. The one deliberately solid object on screen is the primary action.

**3. Approachable precision.** The audience is a solo contractor or a three-person office. Precision belongs in the numbers; copy, labels, and empty states stay plain and human.

**4. Local-first is visible.** The app never implies a cloud. Storage, backup, and export state are shown as facts about the user's machine — a path, a timestamp, a file size.

**5. AI is a proposal, never a fact.** Model output is visually marked as unapplied until accepted, and shown alongside current values rather than in place of them.

**6. Keyboard and screen reader first on the list.** Every record and field is reachable without a pointer. Nothing exists only in a visual affordance.

**7. Density is a setting, not a default opinion.** Row height is tokenized; comfortable and compact differ by one variable.

---

## 2. Color tokens

Roles carry a 100–900 ramp generated in OKLCH on one shared lightness scale, so the same step of any role has the same visual weight. Use 100–300 for tinted fills, hovers, and subtle borders; 500 as the base; 700–900 for text on tinted fills and pressed states. Prefer ramp steps over ad-hoc `color-mix()`.

### Base roles (light)

```css
:root {
  --color-bg: #f2f2f3;
  --color-surface: #e9e9ea;
  --color-text: #1d1f20;
  --color-accent: #5980a6;
  --color-divider: color-mix(in srgb, #1d1f20 16%, transparent);
}
```

### Ramps

```css
:root {
  --color-neutral-100: #f5f5f8;  --color-neutral-200: #e7e7ea;
  --color-neutral-300: #d4d4d7;  --color-neutral-400: #b7b7ba;
  --color-neutral-500: #98989b;  --color-neutral-600: #7a7a7d;
  --color-neutral-700: #5d5d60;  --color-neutral-800: #424244;
  --color-neutral-900: #2b2b2d;

  --color-accent-100: #eef6ff;   --color-accent-200: #d6ebff;
  --color-accent-300: #b5d9fd;   --color-accent-400: #94bce3;
  --color-accent-500: #749dc4;   --color-accent-600: #597ea3;
  --color-accent-700: #416180;   --color-accent-800: #2c455d;
  --color-accent-900: #1d2d3d;
}
```

The palette is mono: one steel accent. `--color-accent-2-*` exists in the system as a machine-derived stand-in that resolves to the same role — treat it as accent. Do not introduce a second brand hue.

### Semantic aliases

ContractorCRM does not use the scheduling aliases (`--sched-*`). It defines its own thin layer over the same ramps, and nothing else:

```css
:root {
  --state-proposed:    var(--color-accent-600); /* unapplied AI value */
  --state-selected-bg: var(--color-accent-100);
  --state-focus-ring:  var(--color-accent);
  --state-attention:   var(--color-accent-800); /* stale, overdue, blocked */
}
```

**Never encode meaning in hue alone.** The palette is monochromatic by design, so every status carries a second channel: a border weight, a glyph, or a text label.

### Contrast rules

The accent-to-ground pair is tuned to ≥3:1 — enough for icons, chrome, and large text, not for body copy. For paragraph-size accent text on the light ground use `--color-accent-700`. Muted metadata bottoms out at `--color-neutral-700`; never `-500` or lighter for text.

---

## 3. Type tokens

```css
:root {
  --font-heading: "Barlow Condensed", system-ui, sans-serif;
  --font-heading-weight: 600;
  --font-body: "Barlow", system-ui, sans-serif;
  --font-numeric: "Barlow", system-ui, sans-serif; /* tabular figures */
}
```

Barlow Condensed sets headings, panel titles, column headers, and the wordmark. Barlow sets body, labels, and inputs.

| Role | Family | Size | Weight | Notes |
| --- | --- | --- | --- | --- |
| View title | Condensed | 24px | 600 | uppercase optional, tracking +0.02em |
| Panel title | Condensed | 15px | 600 | uppercase, tracking +0.08em |
| Column header | Condensed | 12px | 600 | uppercase, tracking +0.08em |
| Table cell | Barlow | 13px | 400 | line-height 1.25 |
| Numeric cell | Barlow | 13px | 500 | `font-variant-numeric: tabular-nums`, right-aligned |
| Label | Barlow | 12px | 500 | |
| Metadata | Barlow | 11px | 400 | `--color-neutral-700` |
| Body prose | Barlow | 14px | 400 | line-height 1.5, `text-wrap: pretty`, max 68ch |

All dates, durations, currency, and float values use `font-variant-numeric: tabular-nums` so columns align across rows. Currency is rendered from integer minor units; the UI never does float math.

### Spacing, radius, elevation

Density 0.85× and 4px radius are baked in. Use the variables, not raw px.

```css
--space-1: 3.4px;  --space-2: 6.8px;  --space-3: 10.2px;
--space-4: 13.6px; --space-6: 20.4px; --space-8: 27.2px;

--radius-sm: 2px;  --radius-md: 4px;  --radius-lg: 7px;

--shadow-sm: 0 1px 2px color-mix(in srgb, #2b2b2d 14%, transparent);
--shadow-md: 0 3px 10px color-mix(in srgb, #2b2b2d 16%, transparent);
--shadow-lg: 0 12px 32px color-mix(in srgb, #2b2b2d 22%, transparent);
```

Elevation is for dialogs and popovers only. Working surfaces are flat and separated by hairlines.

### Row rhythm

Record lists follow the same rhythm as the sibling module's schedule tables, so the two apps feel like one product:

```css
--row-h: 28px;          /* comfortable — default */
--row-h-compact: 24px;
```

Names, companies, and free text are left-aligned. Dates, amounts, and counts are right-aligned with `font-variant-numeric: tabular-nums`. Currency renders from integer minor units; the UI never does float math.

### Spacing, radius, elevation

Identical to the sibling module — the values in the shared token sheet, used through the variables. Elevation is for dialogs and popovers only; working surfaces are flat and separated by hairlines.

---

## 4. Dark mode

Identical mechanism and values to ContractorProject: a token override on `[data-theme="dark"]`, ground `#1f2124`, surface `#2b2b2d`, text `#ececed`, accent base moving to `#94bce3`. Ramps do not invert; usage inverts. Elevation on dark is a hairline edge plus ambient darkness, never a lighter fill alone. Follow the OS by default with an explicit System / Light / Dark override, and ship both themes at parity.

---

## 5. Iconography

Lucide, stroke-width **1.5**, `currentColor`, never filled. 14px in rows, 16px in buttons and panel headers, 20px in the toolbar. Vendored as inline SVG — the app works offline. Every icon has a visible label or an `aria-label`; icon-only buttons also carry a tooltip.

Where a concept exists in both modules it uses the same glyph — `folder` for a job, `users` for a crew, `tag` for a cost code, `sparkle` for the assistant, `hard-drive` for local storage. CRM-specific additions:

| Meaning | Lucide icon |
| --- | --- |
| Contact | `user` |
| Company | `building-2` |
| Pipeline stage | `columns-3` |
| Deal / opportunity | `handshake` |
| Estimate sent | `file-text` |
| Call | `phone` |
| Email | `mail` |
| Site visit | `map-pin` |
| Follow-up due | `clock` |
| Won / lost | `check` / `x` |

---

## 6. Logo usage

The ContractorCRM mark is **three descending columns with two connector stubs** — a pipeline with work moving down it. It is the sibling of the ContractorProject mark: same 32-unit grid, same 6u member, same 3u gutter, same 1.6u connector, axis rotated. Drawn in every application in `ContractorCRM Logo.dc.html`.

### The mark

| Element | Geometry |
| --- | --- |
| Column 1 | x 3, y 6, w 6, h 22 — ink or paper |
| Column 2 | x 12, y 11, w 6, h 17 — **accent** |
| Column 3 | x 21, y 17, w 6, h 11 — ink or paper |
| Connector A | x 9, y 11, w 3, h 1.6 — accent |
| Connector B | x 18, y 17, w 3, h 1.6 — accent |

Column width 6u, gutter 3u, connectors 1.6u tall bridging the gutter. All three columns sit on a common 28u baseline and run 22u, 17u, 11u — a descending step. The accent carries the middle column and both connectors: the movement is the colored element, the stages are not.

The mark occupies **60% of the tile edge**, optically centered. Tile radius is **22% of the edge on macOS** and **0 everywhere else**.

### Two forms

- **Horizontal lockup** — tile plus the full wordmark, gap equal to 1/3 of the tile edge. The primary application.
- **Square** — tile and mark alone, no name.

Both forms exist on the light ground and the dark ground; neither is a recolor of the other.

### Construction rules

- **Clear space:** the wordmark's cap height on all sides (lockup), or 1/6 of the edge (square). Nothing enters it.
- **Minimum sizes:** lockup 120px wide on screen, 1in in print. Square 24px.
- **Optical sizing:** the icon is drawn per size, not scaled. At **16px the mark drops to two columns with no connectors**. Every size from 32px up keeps the full mark. Ship 16 / 32 / 48 / 128 / 256 / 512 / 1024.
- **Wordmark:** Barlow Condensed 600, tracking −0.012em, set as one word — `Contractor` in ink, `CRM` in the accent. `CRM` stays in caps; it is an abbreviation, not a styling choice.
- **Permitted color pairs**, one accent plus one ink per instance:
  1. Steel field `#1d2d3d` · paper mark `#f2f2f3` · accent `#94bce3` — default, light UI
  2. Ink field `#1d1f20` · paper mark, single color — one-color print, favicons, embossing
  3. Paper field `#ececed` · ink mark `#1d2d3d` · accent `#5980a6` — default, dark UI
  4. Accent-900 field carrying reversed type — installer and marketing banners
- **Registration marks** (the system's `+` crosshairs) belong to the layout, not the logo.

### Suite consistency

Across modules the tile, the field colors, the wordmark face, the `Contractor` prefix in ink, and the suffix in accent are fixed. Only the figure inside the tile changes, and it must be built from the same 6u members, 3u gutters, and 1.6u connectors. A module mark that needs a curve, a diagonal, or a third color does not belong to this suite.

### Don't

- No hue outside the steel palette.
- No stretching or condensing the tile, mark, or lockup.
- No outlined tile, no drop shadow, no gradient.
- No rotation. The columns are plumb.
- No mark on a photograph without a solid field behind it.
- No rebuilding the wordmark in another face, and no tagline inside the clear space.

---

## 7. Open items

- CRM-specific surface specs — contact and company records, the pipeline board, the activity timeline, list and detail layouts — are not written yet. They need a product brief first; nothing here should be read as a decision about them.
- ~~Whether the pipeline board is a column view, a table with a stage column, or both.~~ Decided 2026-08-14: the table with a stage column is the primary pipeline view; a read-only kanban board ships as a summary view (click a card to open the deal), with drag-to-move deferred past v1.
- Comfortable vs. compact as the shipped default row height, decided once for the whole suite.
- The third module's mark, which must be drawn from the same members before the suite grammar can be called settled.
