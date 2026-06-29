# Sudokah

A Sudoku player/solver built with [egui](https://github.com/emilk/egui) / eframe,
compileable for both desktop and Android.

## Project layout

All app logic lives in the library (`src/lib.rs`) so it can be shared by two thin
entry points:

- `src/main.rs` → desktop binary, calls `sudokah::run_desktop()`.
- `android_main` in `src/lib.rs` (behind `#[cfg(target_os = "android")]`) → the
  symbol `android-activity` invokes on Android.

## Desktop

```sh
cargo run            # debug
cargo build --release
cargo test
```

## Android

Requires the Android SDK + NDK and [`cargo-apk`](https://crates.io/crates/cargo-apk):

```sh
rustup target add aarch64-linux-android   # (and others you want to ship)
cargo install cargo-apk
```

Point cargo-apk at your SDK/NDK (paths will differ):

```sh
export ANDROID_HOME=$HOME/Android/Sdk
export ANDROID_NDK_ROOT=$ANDROID_HOME/ndk/<version>
```

Then build / run / package. **Always pass `--lib`** — the APK is the `cdylib`
library (it contains `android_main`); without `--lib`, cargo-apk also tries to
package the desktop `[[bin]]` and panics with "Bin is not compatible with Cdylib":

```sh
cargo apk build --lib                # debug APK under target/debug/apk/, auto-signed
cargo apk run --lib                  # build, install, and launch on a connected device
cargo apk build --lib --release      # release APK (needs a signing keystore, see below)
```

Notes:

- `--lib` is required (see above). The crate builds as both `rlib` (for the
  desktop bin) and `cdylib` (what cargo-apk packages).
- `target_sdk_version` in `Cargo.toml` must be an SDK platform you actually have
  installed (check `$ANDROID_SDK_ROOT/platforms`), or cargo-apk errors with
  "Platform `N` is not installed."
- The Android build uses the `wgpu` renderer with eframe's `accesskit` feature
  **off** — `accesskit` is incompatible with `android-native-activity`. The two
  platforms therefore declare `eframe` under separate `[target.…]` sections in
  `Cargo.toml` so their feature sets don't unify.
- Manifest defaults (package id, label, SDK levels, build target) live under
  `[package.metadata.android]` in `Cargo.toml`; adjust as needed.
- `--release` requires a signing keystore. Generate one and point Cargo at it:

  ```sh
  keytool -genkey -v -keystore release.keystore -alias sudokah \
      -keyalg RSA -keysize 2048 -validity 10000
  ```

  ```toml
  [package.metadata.android.signing.release]
  path = "release.keystore"
  keystore_password = "…"
  ```
