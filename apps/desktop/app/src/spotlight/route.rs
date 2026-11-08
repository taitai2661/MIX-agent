//! Receiving a Spotlight result click.
//!
//! tao installs its own `NSApplicationDelegate`; it answers browsing-web
//! activities and returns `false` for everything else, including Core
//! Spotlight's `CSSearchableItemActionType`. We keep that delegate (and its
//! Handoff handling) and only replace the two method implementations we need,
//! forwarding every other activity to the original IMP.

use std::sync::{Arc, OnceLock};

use objc2::runtime::{AnyObject, Bool, Imp, Sel};
use objc2::{msg_send, sel};
use objc2_app_kit::NSApplication;
use objc2_core_spotlight::{CSSearchableItemActionType, CSSearchableItemActivityIdentifier};
use objc2_foundation::{NSString, NSUserActivity};
use tauri::{AppHandle, Manager};

use super::SpotlightService;

type WillContinueImp =
    unsafe extern "C-unwind" fn(&AnyObject, Sel, *mut AnyObject, &NSString) -> Bool;
type ContinueImp = unsafe extern "C-unwind" fn(
    &AnyObject,
    Sel,
    *mut AnyObject,
    &NSUserActivity,
    *mut AnyObject,
) -> Bool;

static APP: OnceLock<AppHandle> = OnceLock::new();
static SERVICE: OnceLock<Arc<SpotlightService>> = OnceLock::new();
static ORIGINAL_WILL: OnceLock<WillContinueImp> = OnceLock::new();
static ORIGINAL_CONTINUE: OnceLock<ContinueImp> = OnceLock::new();

/// Install the two method replacements. Must run on the main thread; returns a
/// human-readable reason when the delegate or a selector is not the one tao
/// installed, so the settings pane can disable the feature instead of
/// pretending it works.
pub fn install(app: &AppHandle, service: Arc<SpotlightService>) -> Result<(), String> {
    if ORIGINAL_CONTINUE.get().is_some() {
        return Ok(());
    }
    let mtm = objc2::MainThreadMarker::new()
        .ok_or_else(|| "Spotlight routing must be installed on the main thread".to_string())?;
    let application = NSApplication::sharedApplication(mtm);
    let delegate = application
        .delegate()
        .ok_or_else(|| "the application has no NSApplicationDelegate".to_string())?;
    let delegate: &AnyObject = delegate.as_ref();
    let class = delegate.class();

    let will_method = class
        .instance_method(sel!(application:willContinueUserActivityWithType:))
        .ok_or_else(|| {
            "tao's delegate no longer implements willContinueUserActivity".to_string()
        })?;
    let continue_method = class
        .instance_method(sel!(application:continueUserActivity:restorationHandler:))
        .ok_or_else(|| "tao's delegate no longer implements continueUserActivity".to_string())?;

    let _ = APP.set(app.clone());
    let _ = SERVICE.set(service);

    unsafe {
        let original = will_method
            .set_implementation(std::mem::transmute::<WillContinueImp, Imp>(will_continue));
        let original: WillContinueImp = std::mem::transmute(original);
        let _ = ORIGINAL_WILL.set(original);

        let original = continue_method.set_implementation(std::mem::transmute::<ContinueImp, Imp>(
            continue_user_activity,
        ));
        let original: ContinueImp = std::mem::transmute(original);
        let _ = ORIGINAL_CONTINUE.set(original);
    }

    tracing::info!("Spotlight activity routing installed");
    Ok(())
}

unsafe extern "C-unwind" fn will_continue(
    this: &AnyObject,
    selector: Sel,
    application: *mut AnyObject,
    activity_type: &NSString,
) -> Bool {
    if activity_type.isEqualToString(CSSearchableItemActionType) {
        return Bool::YES;
    }
    match ORIGINAL_WILL.get() {
        Some(original) => original(this, selector, application, activity_type),
        // Should be unreachable (the method is installed together with the
        // original), but returning `false` matches tao's behaviour for an
        // unknown activity rather than claiming support we cannot provide.
        None => Bool::NO,
    }
}

unsafe extern "C-unwind" fn continue_user_activity(
    this: &AnyObject,
    selector: Sel,
    application: *mut AnyObject,
    activity: &NSUserActivity,
    restoration_handler: *mut AnyObject,
) -> Bool {
    let activity_type = activity.activityType();
    if activity_type.isEqualToString(CSSearchableItemActionType) {
        if let Some(item_id) = spotlight_identifier(activity) {
            tracing::debug!(%item_id, "Spotlight result clicked");
            if let (Some(app), Some(service)) = (APP.get(), SERVICE.get()) {
                let app = app.clone();
                let service = service.clone();
                // The delegate callback is synchronous; do not make Spotlight
                // wait for SQLite. The service resolves the target and emits
                // the UI event on Tauri's runtime.
                tauri::async_runtime::spawn(async move {
                    service.open_item(&app, item_id).await;
                });
                return Bool::YES;
            }
        }
        // A malformed/unknown identifier is still a Spotlight activity: claim
        // it so the system does not fall through to an unrelated handler.
        return Bool::YES;
    }

    match ORIGINAL_CONTINUE.get() {
        Some(original) => original(this, selector, application, activity, restoration_handler),
        None => Bool::NO,
    }
}

fn spotlight_identifier(activity: &NSUserActivity) -> Option<String> {
    let user_info = activity.userInfo()?;
    let value: *mut AnyObject =
        unsafe { msg_send![&*user_info, objectForKey: CSSearchableItemActivityIdentifier] };
    if value.is_null() {
        return None;
    }
    let value = unsafe { &*value };
    let identifier = value.downcast_ref::<NSString>()?;
    let identifier = identifier.to_string();
    if identifier.is_empty() {
        None
    } else {
        Some(identifier)
    }
}

/// Bring the window forward before the frontend navigates. Kept here rather
/// than in the service so the delegate path and the WebView path share one
/// definition of "show the app".
pub fn show_main_window(app: &AppHandle) {
    if let Some(window) = app.get_webview_window("main") {
        let _ = window.show();
        let _ = window.unminimize();
        let _ = window.set_focus();
    }
}
