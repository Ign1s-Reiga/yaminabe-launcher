# Changelog

Notable changes to Yaminabe Launcher. The launcher reads this file directly, so
each release is a `## <version> — <date>` heading followed by bullet points.

## 0.2.0 — 2026-09-07

- Install a modpack from a `.zip` or `.mrpack` you already have, by choosing the
  file or dropping it onto the import step.
- Search and install modpacks from Modrinth, alongside CurseForge.
- Upgrade a Modrinth modpack in place. Saves and configs are kept, a mod you
  turned off stays off, and files the new version drops are removed.
- Fetch a pack's files a few at a time, counting them off as they land.
- Name a pack's mods after the projects they come from, so the Mods tab shows
  what a mod is rather than the name of its jar.

## 0.1.0 — 2026-09-03

- Track where every downloaded file came from, and record the mods of an
  instance in a per-instance mod list.
- Manage mods on manually created non-Vanilla instances: search, add, and
  enable or disable them from the Mods tab.
- Upgrade a CurseForge modpack in place, downloading only what changed and
  leaving saves, configs and hand-added mods untouched.
- Recover a file that could not be downloaded automatically by linking your own
  copy, with a shortcut to its download page.
- Search Modrinth alongside CurseForge.
- Replace the running and install sidebars with a single activity dock.
- Sign in with a Microsoft account.
