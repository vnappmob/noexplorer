//! Dragging files out of NoExplorer into other apps: a browser's upload box, Finder, Mail (ADR-053).
//!
//! The webview can't hand real files to other apps through HTML5 drag & drop (and the app doesn't
//! use HTML5 DnD at all, ADR-003). So the in-app drag (mouse events, `dnd.ts`) hands over here when
//! the pointer leaves the window with the button still down, and `drag_out` starts a native drag
//! session with the files' URLs, like a drag that began in Finder.
//!
//! Remote files are downloaded into the temp folder first (`drag_out_prepare`, wiped at launch with
//! the rest of `temp_root`): browsers only accept files that already exist; file promises (download
//! on drop) are understood by Finder and Mail only. These are copies, so ADR-006 isn't touched.
//!
//! macOS only for now: our own few lines of AppKit instead of a crate. Windows (COM `IDropSource` +
//! `DoDragDrop`) and Linux (GTK) come once they can be tried on real machines.

use futures::{StreamExt, stream};
use tauri::State;

use crate::jobs::Job;
use crate::state::AppState;
use crate::transfer::{FILE_CONCURRENCY, temp_root};
use crate::util::{file_name, safe_rel};

/// Starts a native drag of `paths` (absolute, existing) from under the pointer.
#[tauri::command]
pub async fn drag_out(window: tauri::WebviewWindow, paths: Vec<String>) -> Result<(), String> {
    let paths: Vec<String> = paths.into_iter().filter(|p| std::path::Path::new(p).is_absolute()).collect();
    if paths.is_empty() {
        return Err("Nothing to drag".into());
    }
    #[cfg(target_os = "macos")]
    {
        let (tx, rx) = tokio::sync::oneshot::channel();
        // AppKit wants the main thread; `with_webview` runs there.
        window
            .with_webview(move |wv| {
                let _ = tx.send(mac::start(wv.ns_window(), &paths));
            })
            .map_err(|e| e.to_string())?;
        rx.await.map_err(|_| "The drag didn't start".to_string())?
    }
    #[cfg(not(target_os = "macos"))]
    {
        let _ = (window, paths);
        Err("Dragging files out isn't available on this system yet".into())
    }
}

/// Most a drag-out downloads before it starts; past this, a normal download is the better tool.
pub const DRAG_OUT_MAX: u64 = 200 * 1024 * 1024;

/// Downloads remote files and folders (keys ending in `/`, with everything below them) into a fresh
/// temp folder for `drag_out`; returns the local path of each item, in order. Folders are listed
/// first, so going over [`DRAG_OUT_MAX`] is refused before anything downloads.
#[tauri::command]
pub async fn drag_out_prepare(
    state: State<'_, AppState>,
    account: Option<String>,
    bucket: String,
    keys: Vec<String>,
) -> Result<Vec<String>, String> {
    let c = state.remote(account.as_deref())?;
    let dir = temp_root().join(format!("drag-{}", uuid::Uuid::new_v4()));
    let mut tops = vec![];
    let mut files: Vec<(String, std::path::PathBuf)> = vec![];
    let mut total = 0u64;
    for key in keys {
        let top = dir.join(safe_rel(&file_name(&key)));
        if key.ends_with('/') {
            // An empty folder still has to exist to be dragged.
            std::fs::create_dir_all(&top).map_err(|e| e.to_string())?;
            for (k, size) in c.list_all(&bucket, &key).await?.into_iter().filter(|(k, _)| !k.ends_with('/')) {
                total += size;
                files.push((k.clone(), top.join(safe_rel(&k[key.len()..]))));
            }
        } else {
            files.push((key, top.clone()));
        }
        tops.push(top.to_string_lossy().into_owned());
    }
    if total > DRAG_OUT_MAX {
        let _ = std::fs::remove_dir_all(&dir);
        return Err(format!(
            "Over {} MB to drag out: download it first, then drag it from the left pane",
            DRAG_OUT_MAX >> 20
        ));
    }
    let job = Job::untracked("open", format!("{} files", files.len()), c.account(), &bucket, "");
    let results: Vec<Result<(), String>> = stream::iter(files)
        .map(|(key, path)| {
            let (c, job, bucket) = (&c, &job, &bucket);
            async move { c.download_file(bucket, &key, &path, job).await }
        })
        .buffer_unordered(FILE_CONCURRENCY)
        .collect()
        .await;
    if let Some(Err(e)) = results.into_iter().find(Result::is_err) {
        let _ = std::fs::remove_dir_all(&dir);
        return Err(e);
    }
    Ok(tops)
}

#[cfg(target_os = "macos")]
mod mac {
    use std::cell::RefCell;
    use std::ffi::c_void;

    use objc2::rc::Retained;
    use objc2::runtime::{AnyObject, NSObject, NSObjectProtocol, ProtocolObject};
    use objc2::{AllocAnyThread, MainThreadMarker, MainThreadOnly, define_class, msg_send};
    use objc2_app_kit::{
        NSDragOperation, NSDraggingContext, NSDraggingItem, NSDraggingSession, NSDraggingSource, NSEvent,
        NSEventModifierFlags, NSEventType, NSWindow, NSWorkspace,
    };
    use objc2_foundation::{NSArray, NSPoint, NSProcessInfo, NSRect, NSSize, NSString, NSURL};

    define_class!(
        // Copy only, and only to other apps: dropping back on our own window does nothing (the
        // in-app drag already handles that).
        #[unsafe(super(NSObject))]
        #[thread_kind = MainThreadOnly]
        #[name = "NoExplorerDragSource"]
        struct Source;

        unsafe impl NSObjectProtocol for Source {}

        unsafe impl NSDraggingSource for Source {
            #[unsafe(method(draggingSession:sourceOperationMaskForDraggingContext:))]
            fn mask(&self, _session: &NSDraggingSession, context: NSDraggingContext) -> NSDragOperation {
                if context == NSDraggingContext::OutsideApplication {
                    NSDragOperation::Copy
                } else {
                    NSDragOperation::None
                }
            }
        }
    );

    impl Source {
        fn new(mtm: MainThreadMarker) -> Retained<Self> {
            let this = Self::alloc(mtm).set_ivars(());
            unsafe { msg_send![super(this), init] }
        }
    }

    thread_local! {
        /// The running session's source, kept alive until the next drag replaces it.
        static SOURCE: RefCell<Option<Retained<Source>>> = const { RefCell::new(None) };
    }

    const ICON: f64 = 48.0;

    pub fn start(ns_window: *mut c_void, paths: &[String]) -> Result<(), String> {
        let mtm = MainThreadMarker::new().ok_or("Not on the main thread")?;
        // A drag that starts after the button came up would drop wherever the pointer is.
        if NSEvent::pressedMouseButtons() & 1 == 0 {
            return Err("The mouse button was released".into());
        }
        if ns_window.is_null() {
            return Err("No window".into());
        }
        // SAFETY: Tauri hands out the webview's live NSWindow, and we're on the main thread.
        let window: &NSWindow = unsafe { &*(ns_window as *const NSWindow) };
        let view = window.contentView().ok_or("No window view")?;
        let at_window = window.mouseLocationOutsideOfEventStream();
        let at = view.convertPoint_fromView(at_window, None);

        let ws = NSWorkspace::sharedWorkspace();
        let items: Vec<Retained<NSDraggingItem>> = paths
            .iter()
            .enumerate()
            .map(|(i, p)| {
                let path = NSString::from_str(p);
                let url = NSURL::fileURLWithPath(&path);
                let item =
                    NSDraggingItem::initWithPasteboardWriter(NSDraggingItem::alloc(), ProtocolObject::from_ref(&*url));
                let icon = ws.iconForFile(&path);
                icon.setSize(NSSize::new(ICON, ICON));
                // A small fan for several files, like Finder.
                let off = i.min(4) as f64 * 6.0;
                let frame = NSRect::new(
                    NSPoint::new(at.x - ICON / 2.0 + off, at.y - ICON / 2.0 - off),
                    NSSize::new(ICON, ICON),
                );
                let contents: &AnyObject = &icon;
                // SAFETY: an NSImage is a valid dragging-frame content.
                unsafe { item.setDraggingFrame_contents(frame, Some(contents)) };
                item
            })
            .collect();
        let items = NSArray::from_retained_slice(&items);

        // The session needs a mouse-drag event; the one being handled now is an IPC message.
        let event = NSEvent::mouseEventWithType_location_modifierFlags_timestamp_windowNumber_context_eventNumber_clickCount_pressure(
            NSEventType::LeftMouseDragged,
            at_window,
            NSEventModifierFlags::empty(),
            NSProcessInfo::processInfo().systemUptime(),
            window.windowNumber(),
            None,
            0,
            1,
            1.0,
        )
        .ok_or("Couldn't start the drag")?;

        let source = Source::new(mtm);
        let session =
            view.beginDraggingSessionWithItems_event_source(&items, &event, ProtocolObject::from_ref(&*source));
        session.setAnimatesToStartingPositionsOnCancelOrFail(true);
        SOURCE.with(|s| *s.borrow_mut() = Some(source));
        Ok(())
    }
}
