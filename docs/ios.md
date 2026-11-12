# Starling on iOS

An unofficial iOS build of Starling 0.8.0 (see `IOS-SYNC.md` for how it follows Starling). It is
Starling's phone build: the same setup, title screen, control system, accounts, skins, catalog and
Diagnostics as the Android app. GitHub Actions builds it (`.github/workflows/ios.yml`); the IPA is
unsigned, so install it with a sideloading tool that signs it for your device.

What differs from Android, because iOS does not allow it or does it another way:

- **Native mods (`.hnmod`)** cannot be compiled in or loaded: iOS runs no code the app did not
  ship with. The catalog does not offer them, and the Mods page says so.
- **Game files** are downloaded from Mojang and checked as on Android, then laid out from tables
  the app carries instead of being prepared with Java.
- **Your game folder** is the app's Documents: "Starling" under On My iPhone in the Files app.
- **Updates** come as a new IPA, and there is no separate recovery window before the game starts.

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
