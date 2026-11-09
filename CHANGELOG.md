# Changelog

Notable changes to Yaminabe Launcher. The launcher reads this file directly, so
each release is a `## <version> — <date>` heading followed by bullet points.

## 1.2.3 — 2026-11-09

- Show a player name changed on minecraft.net as soon as the launcher has
  checked the account, in the navigation bar and in Settings.
- Keep the sign-in dialog waiting when one of its checks with Microsoft gets
  no clear answer, instead of ending the sign-in. It still ends when the code
  expires.
- Say again, as before 1.2.2, that the sign-in could not start when Microsoft
  turns down the request for a code.
- Keep a launch from putting back an account you removed while it was
  starting, which then could not be used.

## 1.2.2 — 2026-11-09

- Mark an account in the navigation bar's account switcher whose sign-in has
  expired, or which does not own Minecraft: Java Edition, as Settings does.
  Point at the mark to read why. Until now, you found out when a launch failed.
- Check each account's sign-in once when the launcher starts, instead of every
  time you open Settings.
- Keep an account from being marked "Sign-in expired" when the reply that
  turned it down came from a proxy or a gateway, or made no sense, rather than
  from Microsoft or Minecraft. Such a reply now leaves the sign-in as it is.
- Give up on a sign-in request after 30 seconds, so a network that never
  answers can no longer hold a launch or a check for good.
- Keep a new player name even when the launcher cannot write to Windows'
  credential store.

## 1.2.1 — 2026-11-09

- Keep an account in Settings from being marked "Sign-in expired" again just
  after you sign it in again, when the check that Settings started on opening
  came back late.
- Offer "Sign in again" when Microsoft or Xbox Live turn a sign-in down with
  a reason the launcher cannot read. Until now, such a refusal was taken for
  a server that could not be reached, and a launch said to try again later.

## 1.2.0 — 2026-11-09

- Switch the account you play as from the right end of the navigation bar. It
  shows the selected player's head and name, and its menu, which opens upward,
  lists your accounts, adds a new one, or takes you to Settings.
- Mark an account in Settings → Accounts whose sign-in can no longer be
  renewed, and sign it in again from there. A launch that fails for this
  reason now tells you to do so.
- Stop a launch from waiting forever at the sign-in check on a network that
  never answers. After 10 seconds it goes ahead with the saved sign-in.
- Say again why Minecraft's servers turned a sign-in down when signing in or
  renewing fails at its last step. Since 1.1.0, the reason was left out.

## 1.1.0 — 2026-11-08

- Check your Microsoft sign-in with Minecraft's servers before each online
  launch, and renew it when they no longer accept it. Until now, a sign-in
  turned down before it was due to expire was handed to the game, which then
  could not join servers. A new player name is picked up on the way. When the
  servers cannot be reached, the launch goes ahead with the saved sign-in.
- Keep the launcher working when you close the upgrade dialog before its
  versions have loaded. Until now, it could stop responding.
- List each version once in the install, upgrade and add-mod dialogs. When a
  version was published while you scrolled, the last one of a page could show
  up twice.
- Keep a folder that is still installing out of the library when another
  program, such as an antivirus, holds its install marker open. Until now, it
  could show up as an instance that cannot be loaded.

## 1.0.4 — 2026-10-31

- Offer a button to try again in the install, upgrade and add-mod dialogs when
  their first versions fail to load, too. Until now, you had to close the
  dialog and open it again.
- Keep Install greyed out in the install dialog until there is a version to
  install. When none could be loaded, it looked usable but did nothing.

## 1.0.3 — 2026-10-19

- Keep the versions already loaded when loading more of them fails, in the
  install, upgrade and add-mod dialogs, and offer a button to try again.
- Keep the Back button in the add-mod dialog when a mod's versions cannot be
  listed, so you can return to the search.
- Add the mod you picked in the add-mod dialog. If you went back and picked
  another mod while the first one's versions were still loading, its list
  could fill with the first mod's versions, and Add installed that mod.
- Keep saying in the upgrade dialog that an instance is already on the
  newest version while more versions load, or when they fail to load.
- Keep the modpack tiles on Home and among your bookmarks the same size
  whether or not their pictures have loaded.
- Tell you about a folder left by an install that was cut short just as it
  started, as an instance that cannot be loaded, instead of hiding it. Delete
  the folder yourself to free its name.

## 1.0.2 — 2026-10-16

- Keep showing an instance in the library when its folder holds a file named
  .installing. Since 1.0.1, such a file hid the instance.
- Keep the version you picked in the install dialog. If you closed a modpack's
  dialog and opened it again before its versions had loaded, the earlier
  opening's list could reset your pick or show some versions twice.

## 1.0.1 — 2026-10-16

- Stop the library from deleting the folder of an install that has just
  started. If the library loaded at that exact moment, it could take the new
  folder for one left behind by an interrupted install.
- Show a modpack's own versions in its install dialog. If you opened another
  pack before the first one's versions had loaded, the dialog could list them,
  and install one under the second pack's name.

## 1.0.0 — 2026-10-15

- Show a small splash screen while the launcher starts, instead of an empty
  window that fills in as it loads.
- Show instances as cards built around their modpack's icon. A pack you install
  from the search keeps its icon from now on; other instances show their
  initials on the color of their mod loader. Point at a card to play it, read
  on it whether it is installing or running, and switch the library between
  these cards and a list.
- Bookmark a modpack from the search results and come back to it in the
  library, where a switch on the right turns from your instances to your
  bookmarks. A bookmark installs through the same dialog as a search result.
- Clear away the folder an install leaves behind when the launcher closes or
  crashes partway through, so its name is free again. Creating an instance by
  hand now cleans up after itself when it fails, too. Folders left this way by
  earlier versions stay where they are; delete one yourself to free its name.
- Create a new instance from a floating button at the bottom right of the
  library, which shows its label when you point at it.
- Turn the search, install and browse buttons into icons, and give the button
  that removes an account a proper size and a label.
- Show only the notes of the version you are running in What is new.

## 0.6.2 — 2026-10-10

- Tell you when an instance cannot be loaded. If an instance's settings file
  is missing or damaged, the launcher shows a notice at startup naming its
  folder, instead of the instance quietly disappearing from the library. The
  reason goes into the launcher's log, now kept in `%LOCALAPPDATA%\.yaminabe\logs`.
- Remove the Language and Theme settings, which never did anything. The
  launcher keeps following your system's light or dark preference.

## 0.6.1 — 2026-10-09

- Remove a mod's old version when a CurseForge upgrade brings a new version of
  it under the same file name and that download fails. Before, the old version
  kept loading while the Mods tab reported the new one as missing.

## 0.6.0 — 2026-10-09

- Update the launcher from inside it. Settings → Software Update checks for a
  new version, lists what changed, and installs it, restarting into the new
  version. The Settings button shows a dot when one is waiting, and the
  launcher checks once each time it starts.
- Keep a mod you turned off turned off when a modpack upgrade brings a new
  version of it under a different file name, on both CurseForge and Modrinth.
  Before, the new version came back on and your choice was lost.
- Tell a modpack's own `.disabled` files apart from mods you turned off, so an
  upgrade neither switches off a mod the pack means to be on nor deletes a file
  the pack ships beside it.

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
