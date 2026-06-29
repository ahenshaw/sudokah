// Desktop entry point. All app logic lives in the library (`src/lib.rs`) so it
// can be shared with the Android entry point (`android_main`).
fn main() -> eframe::Result<()> {
    sudokah::run_desktop()
}
