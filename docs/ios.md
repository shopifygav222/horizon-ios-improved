# Horizon Pocket Edition (iOS)

Horizon Pocket Edition ("Horizon PE" under its icon) is an unofficial iOS port of Horizon, Minecraft:
Java Edition rewritten in Rust, based on Starling 0.8.0, Horizon's Android port (see `IOS-SYNC.md` for
how it follows Starling). It has Starling's phone features: the same setup, title screen, control system, accounts, skins, catalog and
Diagnostics as the Android app. GitHub Actions builds it (`.github/workflows/ios.yml`); the IPA is
unsigned, so install it with a sideloading tool that signs it for your device.

What differs from Android, because iOS does not allow it or does it another way:

- **Native mods** can't be downloaded into the app and loaded later: iOS runs no code the app
  did not ship with. Instead they are compiled into the app itself (below).
- **Game files** are downloaded from Mojang and checked as on Android, then laid out from tables
  the app carries instead of being prepared with Java.
- **Your game folder** is the app's Documents: "Horizon PE" under On My iPhone in the Files app.
- **Updates** come as a new IPA, and there is no separate recovery window before the game starts.

## Native mods

A native mod changes the game's own code, and iOS runs no code its app did not ship with, so on
iOS a native mod is compiled into the app. GitHub does the compiling; everything else happens in
the game.

**Once: a GitHub token.** On github.com (Safari works), Settings > Developer settings >
Fine-grained tokens > Generate new token, for this repository only, with the permission
**Actions: Read and write**. Copy it. The game keeps it in the phone's Keychain and sends it only
to GitHub, to start builds and follow them.

**Then, in the game:**

1. Options > Mods > **Catalog**: native mods are listed with the content mods, marked
   "(native mod)". Install the ones you want, as on Android.
2. Back on the Mods page, its top says which native mods this app has, and that the ones turned
   on aren't all in it. The first time, copy your token and press **Paste GitHub token**; then
   press **Build on GitHub**.
3. Keep playing. The game follows the build (about 30 to 60 minutes), then downloads it with the
   same token. When it is ready (or if it failed) a toast says so wherever you are, and the title
   screen shows a line under the account button until you install or dismiss it; tapping it
   opens the Mods page. If the app was closed when the build finished, it catches up the next
   time it starts. **See the build** opens it on GitHub. (GitHub can also tell you on your phone:
   github.com > Settings > Notifications > Actions, untick "Only notify for failed workflows", and
   the GitHub app notifies when your build finishes.)
4. Press **Install...**: the share sheet opens with the .ipa; pick **SideStore**, **AltStore**,
   **LiveContainer** or the app you sideload with (**Save to Files** keeps a copy instead). It
   installs over Horizon PE: the same app, so worlds, settings and accounts stay. Options > Mods
   lists the mods that are in it; each turns off there like any other, without building again.
   The downloaded .ipa is deleted once the new app runs, or when you press Dismiss.

A build has every native mod turned on in the folder, and keeps the mods this app already has that
are still on. A mod turned off stops at once; the Mods page then offers a build without it, which
takes it out of the app. With every native mod off, the normal Horizon PE build has none.

## Installed packs and mods

Each catalog shelf (Mods, Resource packs, Shader packs) has an **Installed** button: it lists
everything in that shelf's folders, whether it came from the catalog, the Files app or an import,
with its size and where it came from, and the search box narrows it. **Remove** asks first, then
deletes it (through the catalog when the catalog installed it, so its list stays right). A
removed resource pack or shader pack leaves its page at once; a removed content mod leaves when
the game next starts; a removed native mod also turns off. A native mod compiled into the app has
no file to delete: its row has **Turn off** instead.

**Without the game:** open this repository's **Actions** tab, pick **ios**, then **Run workflow**,
and type the mods' ids in **Native mods to build in** (`wrench fps_overlay coords_hud`). An id is a
folder of `native-mods/` here (Starling's own, reviewed), or a native mod of Horizon's registry
for Minecraft 26.3: its file's name without the version (`fps_overlay` for
`fps_overlay-2.0.0.hnmod`). `python3 tools/ios_native_mods.py list INDEX` lists them all, with the
registry's index.json (https://horizon-rust.github.io/horizon-mods/index.json) as INDEX. The .ipa
is the run's artifact, `horizon-pe-ios-<number>-native-mods`, kept 30 days. Builds are never
published as releases: only people who can read this repository's Actions can download them.

**What the build does:** it checks each mod first (an unknown id stops it within a minute),
downloads it from the registry's own GitHub releases and checks its size and SHA-256, applies it
to Starling's source as Starling's own Android mod builds do (horizon-mixins,
`packaging/ios/native_stage.sh`), compiles the app and starts it once in the iOS Simulator with
the mods in.

**What can go wrong:** registry mods are written for Horizon, and Starling is a different branch
of the same game. Most apply as they are (the build adapts the ones made before Horizon split its
client into crates), but one that hooks or calls something Starling changed fails the build: the
Mods page says it failed, and See the build shows the mod's file in the log. Turn that mod off and
build again. A `.hnmod` you imported yourself that is in neither the registry nor `native-mods/`
can't be fetched by GitHub: the build says which.

Native mods are not sandboxed: their code runs with the app's full access. Only the registry's
reviewed mods and this repository's `native-mods/` can be built in.

## Device checklist

Install the IPA beside any other build, then, in order:

1. **First start:** the setup appears. Sign in (Microsoft, Ely.by, or an offline profile), let it
   download and lay out the game files, and reach the title screen.
2. **Second start:** the title screen comes straight up, with the account still picked.
3. **Single player:** create a world, walk, break and place blocks, open the inventory, leave and
   re-enter the world.
4. **Controls:** open the control hub (H), switch between MCPE and GoyControl, move a button in
   the editor and save, try the virtual mouse in a menu, and the gyroscope if you use it.
5. **Keyboard:** tap the chat, a sign and a search box: the keyboard opens and types; a tap beside
   the field puts it away. Name a control preset.
6. **Multiplayer:** join an online-mode server with a Microsoft account, and an Ely.by server
   with an Ely.by account.
7. **Skins:** Skin Customization shows the profile's skin; change it.
8. **Catalog:** from Resource Packs and Shader Packs, install one of each and turn it on.
9. **Files:** the Horizon PE folder shows in the Files app; open a resource pack zip with Horizon PE
   from Files.
10. **Diagnostics:** Options > Diagnostics, export a ZIP and share it: the Files sheet opens.
11. **Hardware:** a controller, and a keyboard and mouse if you have them.
12. **Endurance:** play 15 minutes in one world, send the app to the background and bring it back.

If anything fails, send the diagnostic ZIP (step 10) or `Horizon PE/logs` from the Files app.
