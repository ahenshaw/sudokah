// Hide the Windows console window in release builds (a GUI app shouldn't spawn
// one), while keeping it in debug builds so `println!`/panics stay visible.
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

// Desktop entry point. All app logic lives in the library (`src/lib.rs`) so it
// can be shared with the Android entry point (`android_main`).
fn main() -> eframe::Result<()> {
    sudokah::run_desktop()
}
