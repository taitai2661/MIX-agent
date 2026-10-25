# UI conventions (統一基準)

The design tokens live in `ui/src/styles/tokens.css`; the shared primitives live in
`ui/src/components/{layout,common,shared}.tsx` and `ui/src/components/ui/*`.
This document is the single source of truth for the *choices* those primitives encode.
When a pane needs one of these roles, it must use the listed primitive or class — never
retype the markup.

## Icon scale

Every lucide icon size in the app is one of these, chosen by the icon's role:

| Context | Size |
| --- | --- |
| App titlebar controls (`App.tsx` chrome) | 16 |
| Sidebar nav rows, dropdown menu items | 15 |
| Panel header icon / close / back, dialog close, `Alert` icon | 14 |
| `Card` title icon, row glyphs, icon-only `Button` (`size="icon-sm"`), `Trash2` | 13 |
| Glyph inside a labelled `Button` (any size), disclosure chevrons, `Spinner` in a button | 12 |
| `Spinner` standalone (component default) | 13 |
| Icon inside a `Badge` / status chip | 10 |
| Micro glyph next to a `text-2xs` label | 11 |
| `EmptyState` icon | 22 |

## Typography roles

| Role | Classes / primitive |
| --- | --- |
| Page title | `text-base font-semibold tracking-tight text-fg` (the `Page` header) |
| Pane section heading | `SectionHeading` (`text-sm font-semibold text-fg`) |
| `Card` title | `Card`'s `title` slot |
| Uppercase group label | `SectionLabel` (`text-2xs uppercase tracking-[0.08em] text-faint`) |
| Row / list title | `text-sm font-medium text-fg` |
| Description, hint, explainer | `text-xs text-muted` |
| Meta, timestamp, count | `text-xs text-faint` (dense rows: `text-2xs text-faint`) |

Never open-code `tracking-[…]` for an uppercase label: use `SectionLabel`.
`Field`'s hint and `ToggleRow`'s hint are both `text-xs text-muted`.

## Surfaces, borders, radius

* A card is `rounded-xl border border-line bg-surface shadow-xs` — always via `Card`.
  Chips, inline code and payload blocks nested inside a card are `rounded-md`.
* Dividers are plain `border-line`. Do not use `border-line/60`, `border-line/70`
  or a full-opacity `border-warning` on a card (`Alert` keeps its own `/25` tone border).
* Panel headers are `PanelHeader` (h-titlebar, `px-3`, close `X` 14). No padding or
  `titleClass` overrides.
* A right panel's scroll body is the exported `PANEL_BODY` constant
  (`p-3 gap-3` + `stable-scroll` so content does not shift when a scrollbar appears).
* Popover/dialog overlay is `bg-scrim` (a token), not a literal `bg-black/…`.

## Read-state ladder

Every list or pane reads top to bottom: **pending → error → empty → content**.

* pending: `ListSkeleton` (card-shaped) or `TableSkeleton` (table-shaped); a popover
  list uses `ListboxSkeleton`.
* error: block-level `ErrorBox`; inline/mutation errors beside their control use
  `ErrorBadge`. Never a bare `<p className="text-danger">` or a pre-stringified
  `<Alert tone="danger">` (that is `ErrorBox`'s body).
* empty: `EmptyState` (icon 22, title, optional sub). In a compact surface (side panel,
  popover, sidebar list) use `EmptyState compact`, which drops the box and renders the
  title at `text-xs`. A filtered list with a one-tap reset uses `SearchEmptyState`.

## Actions

* Destructive icon-only: `DeleteIconButton` — it owns the confirm dialog, the
  `ghost-danger`/`icon-sm` chrome, `Trash2` 13 and the pending `Spinner` 12. If you use
  `useConfirm` directly you must render `{dialog}` (a pane that forgot left its delete
  button silently broken).
* Refresh: `Button variant="ghost" size="sm"` + `RefreshCw` 12 (`Spinner` 12 while pending).
* Create / import (the pane's primary action): `Button variant="primary"` (default md size).
* Export: `Button size="sm"` + `Download` 12.
* Icon-only buttons are always `size="icon-sm"` with a 13px glyph — not `size="sm"`.

## Payload / pre blocks

Tool arguments, JSON dumps and skill bodies share one recipe:

```
rounded-md border border-line bg-inset px-2 py-1.5 font-mono text-xs leading-5 text-muted
```

Code with a copy header is `CodeBlock` (`components/CopyButton.tsx`); fenced markdown
falls back to the `.markdown-body pre` rule in `tokens.css`.

## Buttons, hints and one-offs

* Never restate the focus ring (`focus-visible:outline-…`): the global `:focus-visible`
  rule in `tokens.css` is the only focus ring in the app.
* Never hardcode a colour literal (`bg-white`, `bg-black/…`); every colour is a token,
  including the `Switch` thumb (`bg-on-solid`).
* Fixed panel widths come from a single module constant (see `SIDEBAR_WIDTH`,
  `RIGHT_PANEL_WIDTH`) — do not also state the width in a Tailwind class.
* Arbitrary values (`text-[14px]`, `rounded-[20px]`, `min-h-[64px]`) are banned where a
  token exists (`text-base`, `rounded-2xl`, `min-h-16`).
