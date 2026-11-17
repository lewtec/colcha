# CUE parity corpus

These fixtures record the CUE features that `modot home config dump` resolves.
Each `expected.json` is a projection of one dump and is never hand-edited.
Use the corpus to compare future colcha features against real CUE behaviour.

## Refresh

`refresh.sh` requires `modot` and `jq` on `PATH`.

```text
./refresh.sh          # regenerate every expected.json
./refresh.sh --check  # fail when expected.json is stale
```

The projection keeps only the roots that the fixture contributes.
Prelude data (`browser`, `desktop`, `drivers`, and similar) and absolute paths
stay out of the corpus. `refresh.sh` stores one projection per fixture.

## Layout

```text
<fixture>/
  home/.dotfiles/...   # CUE input; use the fixture as HOME
  expected.json        # projected dump
```

Inspect one fixture by hand:

```text
HOME="$PWD/configdash/home" modot home config dump
```

`modot` discovers `$HOME/.dotfiles/modot.cue` and resolves local modules.
The fixtures use small copies of modules from the real dotfiles.

## Feature checklist

Status meanings: `yes` means colcha covers the feature.
`partial` means colcha covers a narrower or different form.
`no` means colcha has no equivalent.
`yes (resolution stage)` means the authoring layer resolves the feature before
`State`; the crate receives concrete facts. See ADR-0012.

| Feature | Fixture | Evidence | colcha today |
|---------|---------|----------|--------------|
| Module resolution | configdash, base16-gtk, webapp | `modules.<name>.input = "self:modules/..."` | yes (resolution stage) |
| Module root fields | configdash | `dashboard_demo` appears at the top level | yes (resolution stage) |
| External module source | schema | `modules.old.from = "core:place"` | yes (resolution stage) |
| Schema and config unification | configdash | `options.theme.accent` merges type and value | yes (resolution stage) |
| Cross-field references | configdash, base16-gtk, webapp | `values.accent: options.theme.accent` | yes (resolution stage) |
| Defaults | base16-gtk, webapp | `prefer_dark: 1`, `cursor_size: 24`, `browser: null` | yes |
| Constraints | base16-gtk | `string & =~"^#[0-9a-fA-F]{6}$"` | partial |
| Conditionals | base16-gtk | `if options.base16.mode == "dark" { ... }` | yes (resolution stage) |
| Open structs | base16-gtk | `#gtkSettingsShared` ends with `...` | yes (`#[colcha(rest)]`) |
| Optional fields and maps | webapp | `apps?: [string]: {...}`, `profile?` | partial |
| Open lists | webapp | `extra_flags: [...string]` | partial |
| Arbitrary and case-sensitive keys | base16-gtk, schema | `base0A`, `org/gnome/desktop/interface`, `/.config/legacy` | yes |
| Templated files | configdash, base16-gtk | `file.home[...]` entries carry `type` and `values` | no |
| Heterogeneous values | base16-gtk, schema | `values` mixes strings, ints, and nested maps | yes (`Raw`) |
| Closed definitions and unions | schema | `#BackupAction` selects `git_repo_sync` or `archive` | yes (`#[colcha(tag)]`) |
| Lists of structs | schema | `backup.actions` keeps input order | yes (`#[colcha(concat)]`) |
| Comprehensions | schema | `core:place` expands `items` into `file.home` | yes (resolution stage) |
| Struct defaults | schema | `lint.tools.00-go.enable = true` comes from the schema | yes |
| Null values | webapp | `whatsapp.browser = null` | partial |

## Open gaps

The corpus also pins behaviour that colcha does not model yet:

- `Option<T>` treats `null` and absence as the same value.
- `Vec<T>` is keyed by default, so input order is lost; `#[colcha(concat)]` keeps order but cannot retract one item.
- Tagged unions accept named variants only; scalar variants use `Raw`.
- Open structs need an explicit `#[colcha(rest)]` field; a definition that ends in `...` has no direct equivalent.
- References, comprehensions, conditionals, interpolation and module schemas resolve before `State`; the crate itself has no syntax for them. See ADR-0012.
