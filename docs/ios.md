# Starling on iOS

An unofficial iOS build of Starling 0.8.0 (see `IOS-SYNC.md` for how it follows Starling). It is
Starling's phone build: the same setup, title screen, control system, accounts, skins, catalog and
Diagnostics as the Android app. GitHub Actions builds it (`.github/workflows/ios.yml`); the IPA is
unsigned, so install it with a sideloading tool that signs it for your device.

What differs from Android, because iOS does not allow it or does it another way:

- **Native mods** can't be downloaded into the app and loaded later: iOS runs no code the app
  did not ship with. Instead they are compiled into the app itself (below).
- **Game files** are downloaded from Mojang and checked as on Android, then laid out from tables
  the app carries instead of being prepared with Java.
- **Your game folder** is the app's Documents: "Starling" under On My iPhone in the Files app.
- **Updates** come as a new IPA, and there is no separate recovery window before the game starts.

## Native mods

A native mod changes the game's own code, so on iOS it is built into the app. GitHub does the
compiling; you only pick the mods:

1. On github.com (Safari works), open this repository's **Actions** tab, pick **ios**, then
   **Run workflow**. In **Native mods to build in**, type the mods' ids separated by spaces, for
   example `wrench fps_overlay coords_hud`, and run it.
2. An id is a folder of `native-mods/` in this repository (Starling's own, reviewed), or a native
   mod of Horizon's registry for Minecraft 26.3: its file's name without the version, so
   `fps_overlay` for `fps_overlay-2.0.0.hnmod`. `python3 tools/ios_native_mods.py list INDEX` lists
   every one, with the registry's `index.json`
   (https://horizon-rust.github.io/horizon-mods/index.json) as INDEX.
3. The run checks each mod first (an unknown id stops it within a minute), downloads it from the
   registry's own GitHub releases and checks its size and SHA-256, applies it to Starling's source
   the way Starling's own Android mod builds do (horizon-mixins, `packaging/ios/native_stage.sh`),
   compiles the app, and starts it once in the iOS Simulator with the mods in. It takes about as long
   as a normal build.
4. Download the artifact `starling-ios-<number>-native-mods` from the run's page and sideload the
   `.ipa` over Starling: it is the same app, so your worlds, settings and accounts stay. Options >
   Mods lists the mods that are in it; each turns off there like any other.

To change the set of mods, build again with the new list and install that over it. A build without
mods (the normal one) takes them all out again.

What can go wrong: registry mods are written for Horizon, and Starling is a different branch of
the same game. Most apply as they are (the build adapts the ones made before Horizon split its
client into crates), but one that hooks or calls something Starling changed fails the build, and
the run's log names the mod's file. Build again without that mod.

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
9. **Files:** the Starling folder shows in the Files app; open a resource pack zip with Starling
   from Files.
10. **Diagnostics:** Options > Diagnostics, export a ZIP and share it: the Files sheet opens.
11. **Hardware:** a controller, and a keyboard and mouse if you have them.
12. **Endurance:** play 15 minutes in one world, send the app to the background and bring it back.

If anything fails, send the diagnostic ZIP (step 10) or `Starling/logs` from the Files app.
