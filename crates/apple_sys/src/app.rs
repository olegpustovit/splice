unsafe extern "C" {
    fn splice_app_cancel_open_panel();
}

/// Dismisses the attached open panel, if called on the application thread.
pub fn cancel_open_panel() {
    // SAFETY: the shim checks the thread and owns its AppKit references.
    unsafe { splice_app_cancel_open_panel() };
}
