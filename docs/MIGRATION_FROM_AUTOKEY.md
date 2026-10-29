# Migrating from AutoKey

WayExpand can import AutoKey's static phrase files. AutoKey stores phrase text
in a `.txt` file and its settings in a hidden sibling JSON file, such as
`greeting.txt` and `.greeting.json`. Import writes TOML to standard output and
does not modify the source tree.

```sh
wayexpand import autokey "$HOME/.config/autokey/data/My Phrases" \
  --report-json > imported.toml 2> import-report.json
wayexpand validate imported.toml
```

For one phrase pair, pass its `.txt` file instead of the directory.

The importer supports abbreviations and aliases, immediate versus
delimiter-confirmed triggers, case propagation, descriptions, folder
categories, and AutoKey's `<cursor>` marker. It omits Python scripts, hotkeys,
window filters, case-insensitive and inside-word matching, prompted phrases,
custom abbreviation boundary rules, mixed trigger modes, and dynamic macros
that WayExpand cannot faithfully or safely translate.
Permissive mode reports omitted entries; `--strict` stops at the first one.
Review the report before replacing an existing library.

Macros such as `<script>`, `<system>`, `<file>`, and `<date>` are never run
during import. Convert their behavior explicitly after import, then validate
and preview the new snippets.

See the [AutoKey introduction](https://autokey.github.io/intro.html) and
[phrase macro guide](https://github.com/autokey/autokey/wiki/Phrases) for the
source file format and macro behavior.
