# Global search implementation brief

Issue: #16  
Status: ready after #15  
Updated: 2026-08-16

## Boundary

Build a keyboard-first global search surface on issue #15's transactional FTS
seam. Empty search shows recent records and existing favorite contacts. This
issue does not add saved views, tags, CSV, archive support, generalized
favorites, or another search index.

## Application contract

- Reuse #15's `SearchResult` wire shape with `entityType`, canonical
  `recordId`, title, subtitle/snippet, and activity parent navigation fields.
- Extend the Rust application seam and `CoreClient` with recents read/write
  commands. React never reads SQLite directly.
- Store an ordered, capped JSON projection at `navigation.recents.v1` in
  `app_settings`. Deduplicate by entity type + record id, move an opened target
  to the front, cap at 12, and treat a missing setting as an empty list.
- Query favorite contacts from `contacts.favorite = 1` with archived records
  excluded. Do not add favorite fields to companies or opportunities.
- Register new commands through the versioned v1 command registry and update
  `schemas/v1/local-api.json` with the implementation.

## UI boundary

- Add `src/components/GlobalSearch.tsx` and mount it from the app header rather
  than creating another primary tab.
- Centralize record navigation in `App`: contacts, companies, and
  opportunities open their existing detail routes; activity results open the
  activity parent.
- Record a recent only after successful navigation.
- An empty query renders Recent records followed by Favorite contacts. A typed
  query replaces those projections with ranked FTS results.

## Keyboard and accessibility contract

- Command-K on macOS and Control-K elsewhere opens and focuses search.
- Escape closes and restores the invoking focus.
- Arrow Up/Down select; Home/End jump; Enter opens; pointer selection behaves
  identically.
- Use a labelled modal dialog with listbox/option semantics, stable
  `aria-activedescendant`, and a polite result-count/status live region.
- Do not hijack the shortcut from unrelated editable controls.

## Tests

Rust integration coverage:

- search hits and navigation metadata for contact, company, opportunity, and
  activity results;
- archived exclusion and result bounds/order;
- recents absent/deduplicated/reordered/capped and persistent after reopen;
- favorite-contact projection excluding archived contacts.

Frontend coverage:

- platform shortcut and focus restoration;
- result, recents, favorites, empty, and no-result states;
- arrows, Home/End, Enter, Escape, pointer parity, and ARIA state;
- activity-to-parent navigation and recording a recent after navigation.

Real-app acceptance:

1. Create a favorite contact, company, opportunity, and activity, then restart.
2. Open search with Command-K and navigate to every entity using only the
   keyboard.
3. Confirm recents persist in most-recent order and the favorite contact
   appears for an empty query.
4. Confirm Escape returns focus and VoiceOver announces the dialog, input,
   result count, and selected option.
5. Repeat the shortcut and screen-reader pass with Control-K/NVDA on Windows
   before release hardening closes.

## Dependency and risks

- Start only after #15 freezes the search request/result and archive semantics;
  do not duplicate FTS SQL or maintenance.
- Search, recents, and favorites should all exclude archived records. Skip
  stale recent targets rather than surfacing archived or missing records.
- `app_settings` makes recents a replaceable local projection, so no database
  migration is required for #16.
