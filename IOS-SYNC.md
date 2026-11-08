# Starling on iOS: the sync ledger

This branch (`starling-ios` of shopifygav222/horizon-ios-improved) is Starling with an iOS layer and the Horizon Android
features Starling has not taken yet. `main` of the same repository is the Horizon Android iOS port; it stays as it is.

- **Base:** starling-client/starling `starling` @ `7c38f33c1` ("Starling 0.2.3: bring your worlds over from Horizon
  Android"), the same commit as JuanLuci/starling `android`. Remote: `starling`.
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
4. Horizon Android features come from `main` of this repository (the Horizon Android port), unchanged where they
   fit, so a later diff against Horizon Android stays readable.

## Sync procedure

1. At a phase boundary, never in the middle of one: `git fetch starling` and merge `starling/starling` on a
   `sync/<date>` branch.
2. Resolve each conflict with the engine-edit ledger: every row says what the change is and why.
3. Walk the Android gates the merge added (`git diff <old base> <new base> | grep target_os`), and classify each one in
   the gate ledger.
4. `ios.yml` and `ios-guard.yml` green, then merge into `starling-ios` and move **Base** above.
5. When Starling takes one of the features below itself, switch to Starling's and mark the row "upstream"; ours goes
   once Starling's passes the same checks.

## Feature ledger

The Horizon Android features Starling lacks (all approved for this port).

| # | Feature | Phase | Status |
|---|---|---|---|
| 1 | Material You launcher pages | S3 | to do |
| 2 | Catalog: mods, packs, shaders (Horizon index + Modrinth) | S3 | to do |
| 3 | Built-in file manager | S3 | to do |
| 4 | Multiple game profiles | S3 | to do |
| 5 | Launcher settings (renderer, theme, UI scale, cache, workers, GPU profile, experimental) | S3 | to do |
| 6 | Diagnostics ZIP, performance recordings, exit reports | S3 | to do |
| 7 | Ely.by sign-in and account switching | S4 | to do |
| 8 | Wardrobe: local and cloud skins, 3D preview | S4 | to do |
| 9 | GoyControl and MCPE control styles with presets | S5 | to do |
| 10 | Virtual mouse / touchpad | S5 | to do |
| 11 | Dynamic joystick spawn area | S5 | to do |
| 12 | Layout profiles, snap to grid, control hub, shield button | S5 | to do |
| 13 | Keypad profile | S5 | to do |
| 14 | WebAssembly `.hmod` mods (wasmtime's Pulley interpreter on iOS) | S6 | to do |
| 15 | Bundled Smart F3 and recipes mods | S6 | to do |
| 16 | Screen aspect and immersive toggle | S7 | to do |
| 17 | F2 screenshots to Photos | S7 | to do |
| 18 | Game files without Java (bundled tables + verified Mojang downloads) | S2 | to do |

## Engine-edit ledger

Every change to a file Starling has, in the order they were made.

| File | Change | Why |
|---|---|---|

## Gate ledger

Every `target_os = "android"` site in Starling's module tree, and what iOS does there: **widen** (the gate becomes
`any(android, ios)`), **iOS** (an iOS counterpart in the iOS layer), or **Android only** (nothing on iOS needs it).

| File | Site | Class | Note |
|---|---|---|---|
