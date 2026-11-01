# Packages view: design brief

A brief for designing the Packages tab in Claude Design. The backend is done; this lists what the UI gets and the states it has to cover. Field names match `ui/src/bindings`.

## Where it lives

- A new tool-rail entry (view id `packages`) next to Database. Same frame as the other views: 48px header row, content, optional footer with the command preview.
- Forge Dark and Forge Light tokens, Geist / Geist Mono, the existing chips, badges, option cards and `.dialog-foot` footer. The Generator view is the closest model: form on one side, preview on the other, a "Same as `npm install …`" command chip in the header, and a primary action in the footer.

## Data

**Installed** (`api.pkgList()` → `PackageList`, instant, local files only)

- `packageManager` (npm, pnpm, yarn, bun) and `nestVersion` (major, e.g. "12")
- `packages[]`: `name`, `range` (from package.json), `installed` (version in node_modules, or null), `dev`, `essential` (@nestjs/core, rxjs, typescript…)
- `needsInstall`: something in package.json isn't installed yet

**Updates** (`api.pkgLatest()` → `LatestVersion[]`, network, a few seconds)

- `latest`, `update` (major, minor, patch or null)
- `newestForNest`: set when `latest` needs a newer Nest than the project's. Example: on Nest 11, `@nestjs/apollo` latest is 14.1.1, but 13.2.1 is the newest that fits. The UI should offer that version, not latest.

**Nest catalog** (`api.pkgCatalog()` → `CatalogEntry[]`, instant)

About 20 official and common Nest packages grouped by `category` (Essentials, Security, Data, APIs, Background work, Architecture). Each has `title`, `description`, `companions` and `devCompanions` (installed together; e.g. GraphQL → @nestjs/apollo, @apollo/server, @as-integrations/express5, graphql), `installed` and a `docs` link.

**Search** (`api.pkgSearch(query)` → `SearchHit[]`, network, up to 25)

`name`, `version`, `description`, `keywords`, `date`, `weeklyDownloads`, `homepage`, `repository`, `npm` link, and `installed` (the current range when the project already has it).

## Actions

Each action is two steps: a plan, then a run.

1. `api.pkgPlan(action, packages, dev)` → `PackagePlan`. This writes nothing. `action` is `add`, `remove` or `install` (a plain reinstall). For `add`, the backend picks the version line that fits the project's Nest. Each `packages[i]` has the `spec` that will run (`@nestjs/apollo@^13.2.1`) and a `note` explaining it ("13.2.1 is the newest release for Nest 11 (14.1.1 needs a newer Nest)", or "Latest: 4.0.2"). The plan also returns:
   - `peers[]`: required peers the project lacks. Offer them as checkboxes; plan again with the ones the user picks.
   - `command`: the exact command line, to show before running
   - `notes[]`: warnings, e.g. removing an essential package, or moving a package between dependencies and devDependencies
2. `api.pkgRun(action, specs, dev)` starts the command in the background and returns the command line. Output streams to the terminal panel (process `packages`). A `task-done` event with id `packages` and `{ ok, error }` follows, then `project-updated`. Only one runs at a time; a second call fails with "Another install is still running".

Updating a package is `add` with a version: `pkgRun('add', ['@nestjs/config@4.1.0'])`.

## States to design

- Installed list: prod vs dev grouping, installed version vs range, "not installed" rows, essential badge, and update badges (major, minor, patch) once `pkgLatest` returns. Keep the list usable while those load.
- Major update where `newestForNest` differs from `latest` (offer the Nest-compatible version, explain why).
- `needsInstall` banner with a "Run npm install" action.
- Catalog grid with installed state, companions shown before install, and a docs link.
- Search: empty, loading, no results, registry unreachable (offline or a private registry).
- Plan review: specs with notes, peers to opt into, the dev/prod toggle, and the command preview.
- Running: a disabled state while a command runs, with output in the terminal panel; success and failure toasts (the failure text is the tail of npm's output).
- Remove: confirm, with the essential-package warning.
- No project open: the view only exists inside a project.
