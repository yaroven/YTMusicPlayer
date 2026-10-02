//! macOS app events Slint/winit don't surface.

use objc2::{
    AllocAnyThread, define_class, msg_send,
    rc::Retained,
    runtime::{AnyObject, NSObject},
    sel,
};

define_class!(
    // Receives the "reopen" Apple event: the app is opened again (Finder,
    // Dock, Spotlight) while it already runs — e.g. closed to the tray.
    #[unsafe(super(NSObject))]
    #[name = "YtmReopenHandler"]
    struct ReopenHandler;

    impl ReopenHandler {
        #[unsafe(method(handleReopen:withReply:))]
        fn handle_reopen(&self, _event: &AnyObject, _reply: &AnyObject) {
            super::raise();
        }
    }
);

/// Shows the window when the app is opened again while running. Call once
/// the event loop runs (AppKit installs its own handler at launch).
pub fn handle_reopen() {
    let handler: Retained<ReopenHandler> = unsafe { msg_send![ReopenHandler::alloc(), init] };
    // SAFETY: main thread; NSAppleEventManager's documented selector, with
    // a handler method of the matching signature. The handler lives as long
    // as the app (leaked below), as the manager doesn't retain it.
    unsafe {
        let manager: *mut AnyObject =
            msg_send![objc2::class!(NSAppleEventManager), sharedAppleEventManager];
        let _: () = msg_send![
            manager,
            setEventHandler: &*handler,
            andSelector: sel!(handleReopen:withReply:),
            forEventClass: u32::from_be_bytes(*b"aevt"),
            andEventID: u32::from_be_bytes(*b"rapp")
        ];
    }
    std::mem::forget(handler);
}
