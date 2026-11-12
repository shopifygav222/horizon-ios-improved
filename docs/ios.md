# Starling on iOS

An unofficial iOS build of Starling 0.8.0 (see `IOS-SYNC.md` for how it follows Starling). It is
Starling's phone build: the same setup, title screen, control system, accounts, skins, catalog and
Diagnostics as the Android app. GitHub Actions builds it (`.github/workflows/ios.yml`); the IPA is
unsigned, so install it with a sideloading tool that signs it for your device.

The app opens in Horizon Android's launcher: Play, the Mods, Resource Packs and Shaders catalogs,
Installed with a file manager, Settings and Diagnostics. Its accounts page and the title screen's
account button are the same list, so either one adds, removes and picks the profile that plays.
There is no Skins page (Skin Customization in the game changes skins) and no control editor in
the launcher (the control hub in the game has it).

What differs from Android, because iOS does not allow it or does it another way:

- **Native mods (`.hnmod`)** cannot be compiled in or loaded: iOS runs no code the app did not
  ship with. The catalog does not offer them, and the Mods page says so.
- **Game files** are downloaded from Mojang and checked as on Android, then laid out from tables
  the app carries instead of being prepared with Java.
- **Your game folder** is the app's Documents: "Starling" under On My iPhone in the Files app.
- **Updates** come as a new IPA, and there is no separate recovery window before the game starts.

## Device checklist

Install the IPA beside any other build, then, in order:

1. **First start:** the launcher appears. Open the account pill, sign in (Microsoft, Ely.by, or
   an offline profile), press Download, then Play, and reach the title screen.
2. **Accounts, both ways:** on the title screen, the account button shows the profile picked in
   the launcher; pick another there, Quit Game, and the launcher's pill shows that one.
3. **Second start:** the launcher comes up with the account still picked; Play goes straight in.
4. **Launcher pages:** install a mod, a resource pack and a shader pack from their pages, see them
   under Installed, browse the file manager, change the theme and UI scale in Settings.
5. **Single player:** create a world, walk, break and place blocks, open the inventory, leave and
   re-enter the world.
6. **Controls:** open the control hub (H), switch between MCPE and GoyControl, move a button in
   the editor and save, try the virtual mouse in a menu, and the gyroscope if you use it.
7. **Keyboard:** tap the chat, a sign and a search box: the keyboard opens and types; a tap beside
   the field puts it away. Name a control preset.
8. **Multiplayer:** join an online-mode server with a Microsoft account, and an Ely.by server
   with an Ely.by account.
9. **Skins:** Skin Customization shows the profile's skin; change it.
10. **Catalog:** from Resource Packs and Shader Packs, install one of each and turn it on.
11. **Files:** the Starling folder shows in the Files app; open a resource pack zip with Starling
   from Files.
12. **Diagnostics:** Options > Diagnostics, export a ZIP and share it: the Files sheet opens.
13. **Hardware:** a controller, and a keyboard and mouse if you have them.
14. **Endurance:** play 15 minutes in one world, send the app to the background and bring it back.

If anything fails, send the diagnostic ZIP (Options > Diagnostics, or the launcher's Settings > Diagnostics) or `Starling/logs` from the Files app.
