# Starling on iOS: the sync ledger

This branch (`starling-ios` of shopifygav222/horizon-ios-improved) is Starling with an iOS layer: the iOS app is
Starling's phone build, as its Android one. `main` of the same repository is the older Horizon Android iOS port; it
stays as it is.

- **Base:** starling-client/starling `starling` @ `ac874422ed` (tag `v0.8.0`, "the first release of Starling and
  Horizon Android together"). Remote: `starling`. Starling 0.8.0 took Horizon Android's control system, account
  profiles with Ely.by, skins, catalog and Diagnostics itself, so this port carries none of its own: the work on the
  0.2.3 base (a hand port of those) is in history under `archive/starling-ios-0.2.3`.
- **Branch name:** `starling-ios`, not `starling`: Starling's `android.yml` and `ci.yml` run on pushes to `starling`,
  and they need Starling's secrets and runners. Under this name none of Starling's workflows start here; ours are
  `ios.yml` and `ios-guard.yml`.

## Rules

1. iOS code lives in files of its own: `crates/horizon-client/src/ios.rs` and `ios/`, `crates/horizon-ios-game`, and
   `packaging/ios`. Starling's files change only where the game has to call the iOS layer, and every such change is a
   row in the engine-edit ledger below.
2. Starling's `crates/horizon-client/src/lib.rs` keeps `#![cfg_attr(not(target_os = "android"), forbid(unsafe_code))]`
   byte for byte (`packaging/android/native_build.sh` looks for that exact line). Every `unsafe` call into UIKit,
   Foundation, Security or CoreMotion is in `crates/horizon-gamecontroller`, behind safe functions.
3. A behaviour both phones share is gated `any(target_os = "android", target_os = "ios")`; what only Android's JNI
   can do stays `target_os = "android"` and gets an iOS counterpart where the game needs one.
4. The app follows Starling's phone build: where Starling's Android code is not JNI, iOS shares it; nothing is
   added that Starling's Android build does not have.

## Sync procedure

1. At a phase boundary, never in the middle of one: `git fetch starling` and merge `starling/starling` on a
   `sync/<date>` branch.
2. Resolve each conflict with the engine-edit ledger: every row says what the change is and why.
3. Walk the Android gates the merge added (`git diff <old base> <new base> | grep target_os`), and classify each one in
   the gate ledger.
4. `ios.yml` and `ios-guard.yml` green, then merge into `starling-ios` and move **Base** above.
5. When Starling takes one of the features below itself, switch to Starling's and mark the row "upstream"; ours goes
   once Starling's passes the same checks.

## What iOS shares with Starling's Android build, and what it cannot

| Starling 0.8.0 | iOS |
|---|---|
| Control system (MCPE, GoyControl, editor, hub, virtual mouse, keypad, gyroscope) | shared (`game/mobile_controls.rs`); its keyboard and haptics are `mobile_controls/mobile_platform_ios.rs` |
| Accounts (Microsoft, Ely.by, offline profiles), skins per profile | shared; secrets sealed with a Keychain key (`ios.rs`) |
| Catalog (mods, resource packs, shader packs) | shared; native mods are not offered (`starling-catalog`) |
| Diagnostics (Options > Diagnostics, performance recordings, ZIP) | shared; the ZIP is shared through Files' sheet; no exit history or recovery window (Android's `ApplicationExitInfo` and activity) |
| Setup: sign-in, download, prepare | shared; preparing lays out the tables CI made (`horizon-ios-prepare`), since iOS runs no Java |
| Bring worlds over from Horizon Android | shared; the folder is picked in Files |
| Game folder in the Files app | the app's Documents (UIFileSharingEnabled) |
| Native mods (`.hnmod`) compiled on the build server, the library swapped in at the next start | compiled into the app itself: `ios.yml` dispatched with `mods` fetches them (`tools/ios_native_mods.py`), applies them as `native_build.sh` does (`packaging/ios/native_stage.sh`, same horizon-mixins pin and adapt step), builds the .ipa and publishes it (`ios-native-r<run>`); `native-mods.json` beside the program names them. In the game, the catalog lists native mods and the Mods page's top (`game/native_builds_ios.rs`, drawn as Android's card) starts that build with the player's GitHub token, follows it and hands the .ipa to SideStore, AltStore or LiveContainer. iOS runs no code it did not ship with, so there is no swap |
| Folder offer, in-app APK update, architecture APKs, Adreno/Mali/PowerVR paths | Android's own |

## Engine-edit ledger

Every change to a file Starling has, in the order they were made.

| File | Change | Why |
|---|---|---|

## Gate ledger

Every `target_os = "android"` site in Starling's module tree, and what iOS does there: **widen** (the gate becomes
`any(android, ios)`), **iOS** (an iOS counterpart in the iOS layer), or **Android only** (nothing on iOS needs it).

| File | Site | Class | Note |
|---|---|---|---|
