# Synchronizing your snippet library with Git

WayExpand has no account or cloud service. To use the same snippets on several
machines, keep the library in a Git repository and use any remote you like
(GitHub, GitLab, Gitea, a private server) or none at all for local history.
Syncthing or similar tools work too, by syncing the repository directory.

## Set up

```sh
wayexpand sync init --remote git@example.com:you/wayexpand-snippets.git
```

This makes the configuration directory (`~/.config/wayexpand`) a Git
repository that tracks only the library: `expansions.toml` and any
`snippets.d/*.toml` files. Portal tokens, usage statistics, GUI preferences,
and Action Broker configuration are excluded by the generated `.gitignore` and
never leave the machine. Git uses your normal credentials and SSH keys.

When `wayexpand sync` is given a custom configuration path, that file's
filename is tracked as the library's primary configuration instead of
`expansions.toml`. Other machines must use the same filename and point WayExpand
at that file; layered snippets remain in the sibling `snippets.d/` directory.

On another machine, clone the repository into the configuration directory (or
run `sync init` there with the same remote once its own library is empty or
merged).

## Sync

```sh
wayexpand sync          # or Library → Sync library in the GUI
wayexpand sync status
```

Each sync:

1. refuses to commit a library that does not validate;
2. commits local changes;
3. rebases onto the remote;
4. validates the merged library, rolling back to your local version if the
   merge would make it invalid (for example a duplicate trigger);
5. pushes, and asks a running daemon to reload.

A genuine conflict (both machines changed the same snippet) stops before
anything changes and asks you to resolve it with Git in the configuration
directory. Snippets carry stable IDs, so most edits on different machines merge
cleanly. Nothing syncs automatically.
