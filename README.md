<p align="center">
  <img src="https://img.shields.io/badge/Minecraft-Java%2026.3-5D9C3F" alt="Minecraft Java 26.3">
  <img src="https://img.shields.io/badge/platform-iOS%20%26%20iPadOS%2016%2B-1C1C1E?logo=apple&logoColor=white" alt="iOS and iPadOS 16 or newer">
  <img src="https://img.shields.io/badge/graphics-Metal-8E8E93" alt="Metal">
  <a href="https://github.com/starling-client/starling"><img src="https://img.shields.io/badge/built%20on-Starling-2A2470" alt="Built on Starling"></a>
  <a href="https://github.com/ludovic111/horizon"><img src="https://img.shields.io/badge/engine-Horizon-F0795A" alt="Horizon engine"></a>
  <a href="LICENSE"><img src="https://img.shields.io/badge/license-MIT-3B2F7E" alt="MIT license"></a>
</p>

<p align="center">
  <a href="#install"><b>Install</b></a> ·
  <a href="#what-works"><b>What works</b></a> ·
  <a href="#native-mods"><b>Native mods</b></a> ·
  <a href="#build-it-yourself"><b>Build it yourself</b></a> ·
  <a href="docs/ios.md"><b>iOS guide</b></a> ·
  <a href="#credits"><b>Credits</b></a>
</p>

# Horizon Pocket Edition (iOS)

**Minecraft: Java Edition 26.3 on iPhone and iPad, running natively.** Horizon Pocket Edition
("Horizon PE" under its icon) is the iOS port of [Starling](https://github.com/starling-client/starling),
the phone build of [Horizon](https://github.com/ludovic111/horizon): Minecraft's Java game rewritten in
Rust, which reads the same worlds, follows the same rules and generates the same terrain as Java
Edition. There is no Java virtual machine and no OpenGL translation in between: the game draws with
Metal, so it starts fast and plays smoothly on a phone.

It is Starling's phone game, feature for feature (its touch controls, accounts, skins, catalog and
shader packs), with what iOS needs on top: the game files laid out without Java, the Files app,
Photos, the iOS keyboard, Keychain sign-ins, and native mods compiled into the app itself.

> **Not an official Minecraft product.** Not approved by or associated with Mojang or Microsoft.
> Minecraft and "Pocket Edition" are Mojang's trademarks. This app contains no Minecraft code, assets
> or data: it downloads the game's files from Mojang on your phone, and you need your own copy of
> Minecraft: Java Edition 26.3 to play online.

<table>
  <tr>
    <td><img src="docs/media/sunset.jpg" alt="A sunset over a savanna"></td>
    <td><img src="docs/media/village.jpg" alt="A savanna village"></td>
    <td><img src="docs/media/underwater.jpg" alt="Under the sea"></td>
  </tr>
  <tr>
    <td><img src="docs/media/survival.jpg" alt="A survival base"></td>
    <td><img src="docs/media/night.jpg" alt="A night in the hills"></td>
    <td><img src="docs/media/nether.jpg" alt="The Nether"></td>
  </tr>
</table>

<sub>Screenshots of Horizon's game, from the Starling project.</sub>

## Install

**You need** an iPhone or iPad on iOS 16 or newer (iPhone 8 and later), about 2 GB free, and a way
to sideload apps: [SideStore](https://sidestore.io), [AltStore](https://altstore.io),
[Sideloadly](https://sideloadly.io) from a computer, or [LiveContainer](https://github.com/LiveContainer/LiveContainer).

1. **Get the app.** Open this repository's [Actions](../../actions/workflows/ios.yml), pick the latest
   run with a `horizon-pe-ios-<number>` artifact, and download it (GitHub asks you to sign in). Unzip it
   to get `HorizonPE-iOS-unsigned.ipa`. To make a fresh one, press **Run workflow** there with **Make and
   keep the unsigned .ipa** ticked.
2. **Sideload it.** Open the `.ipa` with your sideloading app, or drop it on Sideloadly. The `.ipa` is
   unsigned: the sideloading app signs it with your Apple ID. With a free Apple ID the signature lasts
   7 days; refresh it (SideStore and AltStore do that for you) and nothing is lost.
3. **Set it up.** On the first start, sign in with Microsoft (the account that owns Java Edition),
   Ely.by, or pick an offline profile. The game downloads its files from Mojang, checks each one, and
   lays them out on the phone, once. Then the title screen.

**Updating:** install a newer `.ipa` the same way, over the one you have. It is the same app, so your
worlds, settings, accounts and mods stay. Coming from Horizon's own iOS app? The setup offers to copy
its worlds and resource packs over.

Your game folder is **On My iPhone > Horizon PE** in the Files app: worlds (`saves`), resource packs,
shader packs, mods, screenshots and logs. Open a pack, a world, a skin or a mod (`.zip`, `.starling`,
`.mrpack`, `.hmod`, `.hnmod`, `.png`) with Horizon PE from Files or Safari to add it.

## What works

- **The game:** singleplayer with Java's world generation, mobs, villagers, raids, redstone, every
  command, the HUD and the recipe book, on saves in Java's format. Worlds from older versions are
  upgraded as Java does.
- **Touch controls:** every action on screen, from the movement stick to the hotbar, chat and F3, in
  MCPE or GoyControl style, with presets, a layout editor (move, resize, colour, hide, bind), a control
  hub, a virtual mouse for menus, a keypad, Sneak as Hold or Toggle, and optional gyroscope aiming.
- **Keyboard, mouse and controllers:** a hardware keyboard or mouse takes over the moment you use it;
  game controllers work through iOS's GameController. The on-screen keyboard opens on any text field
  and closes with a tap beside it.
- **Accounts:** Microsoft, Ely.by and offline profiles, several of each, switched from the title
  screen's corner. Sign-ins are kept in the iOS Keychain.
- **Skins:** your profile's skin and cape, and Skin Customization to change them.
- **Catalog:** mods, resource packs and shader packs from Horizon's registry and Modrinth, installed
  with one tap from Options > Mods, Resource Packs or Shader Packs. Each shelf's **Installed** view
  lists everything you have there (from the catalog or the Files app) and removes it.
- **Shader packs:** Iris and OptiFine packs such as Complementary, MakeUp, BSL and Photon, translated
  when they load, with an Auto profile; and **Starling Visuals**, the shader pack made for phones,
  which comes with the app.
- **Servers:** online and offline-mode servers, proxies, server resource packs, plugin servers'
  dialogs, custom items and models, LAN worlds and invites.
- **Made for the phone:** settings picked for your device, a memory governor that keeps the game
  inside what iOS allows, ProMotion up to 120 Hz, screenshots (F2) saved to Photos, and a diagnostic
  ZIP (Options > Diagnostics) to send with a bug report.

### Different from Starling on Android

| Starling (Android) | Horizon PE (iOS) |
| --- | --- |
| A Java runtime prepares the game files on the phone | iOS runs no Java: the app carries the tables Java makes (built by GitHub), and lays the downloaded files out with them, file for file the same |
| Native mods: a library from Starling's build server, swapped in | Native mods are compiled into the app by GitHub Actions ([below](#native-mods)) |
| The `Starling` folder in shared storage | **Horizon PE** in the Files app |
| In-app APK updates | A new `.ipa`, installed over the old one |

## Native mods

Horizon's native mods change the game's own code, and iOS runs no code its app did not ship with, so
on iOS a native mod is compiled into the app. GitHub does the compiling; you pick the mods in the game:

1. Options > Mods > **Catalog**: install native mods, as on Android.
2. The Mods page then offers **Build on GitHub**. The first time, it asks for a GitHub token
   (fine-grained, for this repository only, with **Actions: Read and write**), kept in the Keychain.
3. Keep playing: about 30 to 60 minutes later a toast and a line on the title screen say the build is
   ready. **Install...** opens the share sheet: pick SideStore, AltStore, LiveContainer or your signer.
   Your worlds stay.

Every build is checked before it is installed: each mod is downloaded from the registry's own releases
and checked against its size and SHA-256, applied to the source the way Starling's own mod builds do,
compiled, and started once in the iOS Simulator. Builds stay in the repository's Actions; nothing is
published as a release. You can also start one from the Actions tab with a list of mod ids. All of it,
step by step: [docs/ios.md](docs/ios.md#native-mods).

## Build it yourself

You don't need a Mac: GitHub's Macs build it.

1. **Fork** this repository (keep it public: GitHub's macOS runners are free for public repositories).
2. In your fork, **Actions > ios > Run workflow** with **Make and keep the unsigned .ipa** ticked. About
   40 to 60 minutes later the run has a `horizon-pe-ios-<number>` artifact.

Every push to `starling-ios` (the default branch) runs the same build without keeping the `.ipa`: the game compiled for iOS and
linted, the game files prepared and proved to come out the same on the phone, the app bundled and
checked to carry none of Mojang's files, and the app started in the iOS Simulator
([`ios.yml`](.github/workflows/ios.yml)). [`ios-guard.yml`](.github/workflows/ios-guard.yml) checks that
the iOS work leaves Starling's desktop and Android builds compiling and formatted.

**On a Mac with Xcode:** `rustup target add aarch64-apple-ios`, then `python3 tools/ios_prepare.py`
(with Java 25: it fetches the pinned official 26.3 files, checks them, and makes the game tables the app
carries), then `brew install xcodegen && cd packaging/ios && xcodegen generate` and open
`StarlingIOS.xcodeproj`. Its build phase compiles the Rust game with Cargo and puts it in the app.

## How the repository is laid out

This is Starling's source with an iOS layer; [IOS-SYNC.md](IOS-SYNC.md) records the Starling version
it is built on, every change to Starling's own files, and how to take in a new Starling release.

| Path | What it is |
| --- | --- |
| `crates/horizon-ios-game` | The app's program: its folders, the 16 MB stack the game runs on, the event loop |
| `crates/horizon-gamecontroller` | Every UIKit, GameController, Keychain, CoreMotion and Photos call (the game crates forbid unsafe code) |
| `crates/horizon-client/src/ios.rs` | The game's iOS hooks: files, Keychain sealing, the keyboard, haptics, gyroscope, imports |
| `crates/horizon-client/src/game/native_builds_ios.rs` | Native mods built on GitHub from the Mods page |
| `crates/horizon-ios-prepare` | Lays the game files out on the phone without Java |
| `packaging/ios` | The Xcode project (XcodeGen), `Info.plist`, icon, and the native-mod staging script |
| `tools/ios_prepare.py`, `tools/ios_native_mods.py` | The game tables made in CI; native mods fetched and checked |
| `third_party/winit` | Starling's winit with the iOS patches (scenes, keyboard and mouse, opened files) |
| Everything else | Starling and Horizon, as they are ([Starling's README](https://github.com/starling-client/starling#readme)) |

## Credits

- **[Horizon](https://github.com/ludovic111/horizon)** by Ludovic and its contributors: the game,
  Minecraft: Java Edition rewritten in Rust.
- **[Starling](https://github.com/starling-client/starling)**, formerly Horizon Android, by
  [JuanLuci](https://github.com/JuanLuci) and [Vasysik](https://github.com/Vasysik): the phone build
  this app is, its touch controls, accounts, catalog, skins and Starling Visuals.
- **[horizon-iOS](https://github.com/meehandaniel4/horizon-iOS)**: the first native mod builds for
  iOS on GitHub Actions, which this app's follow.

## License

MIT, as Horizon and Starling: see [LICENSE](LICENSE). Third-party code and its licences are listed in
[THIRD_PARTY_NOTICES.md](THIRD_PARTY_NOTICES.md).
