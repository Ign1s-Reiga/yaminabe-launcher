# Changelog

Notable changes to Yaminabe Launcher. The launcher reads this file directly, so
each release is a `## <version> — <date>` heading followed by bullet points.

## 0.5.0 — 2026-09-07

- Keep a pack's own files and the record of them in step, so an upgrade removes
  what the new version drops and nothing else. A world, a config or an
  `options.txt` that arrived with a pack is yours once you have it, and is left
  alone even when a later version stops shipping it.
- Fetch a pack file that carries no checksum rather than keeping whatever
  already sits at that name, which could leave the previous version installed
  and report it as the new one. A file that cannot be fetched at all is reported
  for you to supply by hand.
- Keep a mod you turned off turned off through an upgrade, whether the pack
  lists it or ships it outright, and whether or not it carries a checksum.
- Refuse a pack path that tries to climb out of the instance or into the
  launcher's own records, on both CurseForge and Modrinth, rather than quietly
  rewriting it into one that stays.
- Offer a usable instance name when importing a pack whose own name cannot be a
  folder, and open the activity dock when an install fails immediately instead
  of appearing to do nothing.

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
