fn main() {
    // Embed the Windows application icon into the .exe (the .rc references
    // assets/icon.ico). embed-resource is a no-op on non-Windows targets, so
    // this is safe to call unconditionally.
    embed_resource::compile("sudokah.rc", embed_resource::NONE);
}
