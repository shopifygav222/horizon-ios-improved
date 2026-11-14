# Horizon Pocket Edition (iOS): the sync ledger

`main` of shopifygav222/horizon-pocket-edition-ios is Starling with an iOS layer: the iOS app,
Horizon Pocket Edition ("Horizon PE" under its icon), is an iOS port of Horizon based on Starling, Horizon's Android
port. The older Horizon
Android iOS port is kept under `archive/horizon-android-ios`, and `main`'s history holds it as a parent; this app was
developed on `starling-ios` until it moved to `main`.

- **Base:** starling-client/starling `starling` @ `ac874422ed` (tag `v0.8.0`, "the first release of Starling and
  Horizon Android together"). Remote: `starling`. Starling 0.8.0 took Horizon Android's control system, account
  profiles with Ely.by, skins, catalog and Diagnostics itself, so this port carries none of its own: the work on the
  0.2.3 base (a hand port of those) is in history under `archive/starling-ios-0.2.3`.
- **Workflows:** only `ios.yml` and `ios-guard.yml`. Starling's own (its APK and release builds, its build service,
  its registry, schedules and pull-request bots) need Starling's secrets and machines, so they are not in this tree:
  a sync that changes one of them is a "deleted here" conflict, kept deleted. So are its `CODEOWNERS`, pull request
  template and native-build issue form; `.github/ISSUE_TEMPLATE/bug_report.yml` is this app's.
- **Name:** the app is Horizon Pocket Edition. `APP_NAME` (render crate, "Horizon PE" on iOS, "Starling" elsewhere)
  is the name the game says; the setup's titles (`game/setup.rs`), the Files-app folder hints (`options_ui.rs`,
  `pack_screen.rs`) and the iOS-only text follow it, and on iOS the language files' "Starling" reads "Horizon PE"
  (`Language::load`), the credits excepted. The app is `HorizonPE.app`, bundle identifier `dev.horizon.pocketedition`, with the
  executable, Xcode project, target and scheme all `HorizonPE` (`packaging/ios/project.yml`). Builds before the
  rename were `Starling.app`, `dev.horizon.starling`: another app to iOS, whose worlds move over in Files.

## Repository layout

Starling's source (Horizon with Starling's Android port) with the iOS layer on top:

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
| Everything else | Starling and Horizon, as they are |

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
4. `ios.yml` and `ios-guard.yml` green, then merge into `main` and move **Base** above.
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
| Native mods (`.hnmod`) compiled on the build server, the library swapped in at the next start | compiled into the app itself: `ios.yml` dispatched with `mods` fetches them (`tools/ios_native_mods.py`), applies them as `native_build.sh` does (`packaging/ios/native_stage.sh`, same horizon-mixins pin and adapt step), builds the .ipa (the run's artifact, never a release); `native-mods.json` beside the program names them. In the game, the catalog lists native mods and the Mods page's top (`game/native_builds_ios.rs`, drawn as Android's card) starts that build with the player's GitHub token, follows it, downloads the artifact with the same token and opens the share sheet (`horizon_gamecontroller::share`) for SideStore, AltStore, LiveContainer or any signer. A build that ends shows a toast (`ServerPackState::notice`) and a title-screen line along the top, clear of the touch pad's buttons (`draw_build_badge`, `build_badge_at` in game.rs's title paths; no-ops elsewhere), which opens the Mods page. The catalog's Installed view (`mod_catalog.rs`) lists and removes everything in a shelf's folders, on Android too. iOS runs no code it did not ship with, so there is no swap |
| Folder offer, in-app APK update, architecture APKs, Adreno/Mali/PowerVR paths | Android's own |

## Engine-edit ledger

Every change to a file Starling has, in the order they were made.

| File | Change | Why |
|---|---|---|
| `horizon-client-render/src/android_controls/layout.rs` | Toggle Sprint and Inventory Walk Controls on by default; `defaults_rev` turns them on once in files saved before | Asked for on iOS; shared with Android |
| `horizon-client-render/src/android_controls.rs` | Over an inventory, the MCPE Sneak button draws a Shift arrow (`shift_glyph`), filled while Shift is down | It is Shift there (quick move) |
| `horizon-client/src/game/mobile_controls.rs` | Sneak latches Shift and Inventory closes over an inventory without the walk controls too; LMB and RMB no longer clear the touch state (Shift, walking); Shift ends with the inventory | Shift stayed on for taps but LMB and RMB dropped it |
| `horizon-client-render/src/gfx/oit.rs` | Improved Transparency asks for `FLOAT32_BLENDABLE` only when `Rgba32Float` itself blends on the adapter | Metal reports the feature without blending the format: the depth-bounds pipeline panicked at world load (M2 iPad, Kelly-Inc/horizon-ios#25) |
| `horizon-client/src/game/native_hooks.rs`, `game.rs` | `native_camera_pose_now` (the frame's camera as native mods see it; handed back unchanged, as this base has no camera-posing hook) and its call where the frame's view is made | The registry's Atlas mod reads the camera at its return; upstream Horizon f319701e8c has it |
| `horizon-client/src/game/native_client_services.rs` (new), `game.rs` | `native_client_profiles`, `native_gui_nine_slice`, `native_send_chat`, as upstream Horizon f319701e8c has them | Host functions Atlas calls; upstream's own file name, so a sync meets them |
| `packaging/android/native_mod_adapt.py` | `render.pixels.get()` becomes `&render.pixels[..]` (CHANGED) | Atlas reads the block textures as newer Horizon keeps them |
| `horizon-client-render/src/dynamic_light.rs` | `select_iter` (`select` over any iterator, appended) | dynamiclights hands its sources from a drain, as newer Horizon has it |
| `horizon-client-core/src/text.rs` | `Text::option_name_value` ("Caption: value" via `options.generic_value`) | dynamiclights' settings screen |
| `horizon-client-render/src/widgets.rs`, `ui.rs` | `widgets::Action` and `ui::action_button` (the plain button here; newer Horizon adds a controller glyph) | dynamiclights' Done button |
| `horizon-client/src/game/native_client_services.rs` | `native_set_mod_key`, `native_client_action_bar`, `native_apply_screen_blur` (false: Options and pause already blur) | dynamiclights' key binding, toggle message and screen |
| `horizon-client/src/mobs/armor.rs` (new), `mobs.rs` | `mobs::armor::values`: armour and toughness as `hurt_after_armor` sums them | Jade's health-and-armour line |
| `packaging/android/native_mod_adapt.py` | Rewrites: burning projectiles from `arrow.on_fire()` (no fire time here), `mob_hit()` as a pair, `key_pressed` with one key, a pet's body armour from `extra.head` | dynamiclights and Jade, pinned to newer Horizon |
| `horizon-client/src/lan/mod.rs`, `lan/conn.rs`, `game/integrated.rs`, `game/dedicated.rs` | The integrated server's own player logs in as the signed-in account's profile (`singleplayer_profile`), as official Horizon 441f49d57e has it | A player is the same player hosting a shared world or joining it (pets, stats, saved state) |
| `horizon-client/src/save.rs` | `save::writes`: every region, `.dat` and saved-file write counted as it starts and ends | A shared world's checkpoint proves nothing wrote while it copied the files |
| `horizon-client/src/title.rs`, `title/tests.rs` | Shared Worlds (`Mode::Shared`, `title/shared.rs`) in the main menu where the disabled Realms button was; `Action::SharedPlay`; shared worlds' folders left out of Singleplayer | Shared worlds |
| `horizon-client/src/game.rs` | The `shared_worlds` module and its fields; a shared world's host takes a checkpoint instead of an autosave; `Action::SharedPlay`; `tick_shared` each frame | Shared worlds |
| `horizon-client/src/game/integrated.rs` | The server waits while a shared world's host can't reach its storage | No progress is made that a takeover would lose |
| `horizon-client/src/game/remote.rs` | A shared world's guest follows the world to its next host instead of the Disconnected screen; `remote_player_list` | Shared worlds |
| `horizon-client/src/game/native_net.rs` | Messages on `shared_worlds:ctl` go to the shared world, not to mods | Ask to Host, reachability |
| `horizon-client/src/game/friends.rs`, `game/saving.rs`, `game/control.rs` | The Friends screen of a shared world; `friends_start_with` (a public tunnel); a checkpoint when the app leaves the screen; `horizon-ctl shared ...` | Shared worlds |
| `horizon-friends/src/portmap.rs`, `host.rs`, `horizon-relay/tests/tunnel.rs` | Without a routing table (iOS, Android) the router is asked at its usual addresses and by unicast SSDP; `HostConfig::tunnel` adds a public tunnel's address to the code when nothing else reaches the host | Phones can host friends on other networks |
| `horizon-friends/src/guest.rs`, `host.rs`, `direct.rs`, `lib.rs`, `reach.rs`; `horizon-client/src/net.rs` | `GuestConfig::tunnels`: the code's tunnel endpoints are dialled after the others had 600 ms; a guest that came through the tunnel (`HZF3`) races the host for a direct stream over it (`session::settle`, as relayed sessions do); `HostEvent::Connected::tunnel`, `Tunnel::tunneled`; `friend:CODE~ADDR` marks them; the code's IPv6 address is the one the device sends from; `net::last_friend_path` | Shared worlds play directly whenever the two ends can meet, bore.pub only when not |
| `horizon-client/src/shared_screen.rs`, `friends_screen.rs` | The shared world's Friends screen shows the invite code with Copy Code beside Done; `friends_screen::code_lines` shared | The code of a shared world's host |

## Gate ledger

Every `target_os = "android"` site in Starling's module tree, and what iOS does there: **widen** (the gate becomes
`any(android, ios)`), **iOS** (an iOS counterpart in the iOS layer), or **Android only** (nothing on iOS needs it).

| File | Site | Class | Note |
|---|---|---|---|
