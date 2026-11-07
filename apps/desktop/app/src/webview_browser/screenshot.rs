//! Platform screenshot adapters for the child WebView.
//!
//! The agent's `screenshot` action still needs a real rendered image. The
//! old Chromium backend used `Page.captureScreenshot`; with a native WebView
//! the platform's own snapshot API is the faithful source.

#[cfg(target_os = "macos")]
pub async fn capture(webview: &tauri::Webview<tauri::Wry>) -> anyhow::Result<Vec<u8>> {
    use objc2::rc::Retained;
    use objc2::runtime::AnyObject;
    use objc2::{AnyThread, MainThreadMarker};
    use objc2_app_kit::{
        NSBitmapImageFileType, NSBitmapImageRep, NSBitmapImageRepPropertyKey, NSImage,
    };
    use objc2_foundation::{NSDictionary, NSError};
    use objc2_web_kit::{WKSnapshotConfiguration, WKWebView};
    use std::sync::Mutex;

    let (tx, rx) = tokio::sync::oneshot::channel::<Result<Vec<u8>, String>>();
    let tx = std::sync::Arc::new(Mutex::new(Some(tx)));

    webview.with_webview(move |platform| {
        let ptr = platform.inner() as *mut WKWebView;
        // SAFETY: `inner()` is the live WKWebView owned by Tauri. Retaining it
        // keeps it valid until the asynchronous completion handler runs.
        let view = match unsafe { Retained::retain(ptr) } {
            Some(view) => view,
            None => {
                if let Some(tx) = tx.lock().ok().and_then(|mut slot| slot.take()) {
                    let _ = tx.send(Err("webview is not available".to_string()));
                }
                return;
            }
        };

        let Some(mtm) = MainThreadMarker::new() else {
            if let Some(tx) = tx.lock().ok().and_then(|mut slot| slot.take()) {
                let _ = tx.send(Err("snapshot must run on the main thread".to_string()));
            }
            return;
        };
        let configuration = unsafe { WKSnapshotConfiguration::new(mtm) };
        unsafe { configuration.setAfterScreenUpdates(true) };
        let sender = tx.clone();
        let handler = block2::RcBlock::new(move |image: *mut NSImage, error: *mut NSError| {
            let result = (|| -> Result<Vec<u8>, String> {
                if !error.is_null() {
                    let message = unsafe { (*error).localizedDescription() };
                    return Err(message.to_string());
                }
                if image.is_null() {
                    return Err("WKWebView returned an empty snapshot".to_string());
                }
                // SAFETY: AppKit hands us a live NSImage for the duration
                // of this callback; retain it while converting.
                let image = unsafe { Retained::retain(image) }
                    .ok_or_else(|| "could not retain the snapshot image".to_string())?;
                let tiff = image
                    .TIFFRepresentation()
                    .ok_or_else(|| "snapshot has no TIFF representation".to_string())?;
                let bitmap = NSBitmapImageRep::initWithData(NSBitmapImageRep::alloc(), &tiff)
                    .ok_or_else(|| "could not decode the snapshot".to_string())?;
                let properties = NSDictionary::<NSBitmapImageRepPropertyKey, AnyObject>::new();
                let png = unsafe {
                    bitmap
                        .representationUsingType_properties(NSBitmapImageFileType::PNG, &properties)
                }
                .ok_or_else(|| "could not encode the snapshot as PNG".to_string())?;
                Ok(png.to_vec())
            })();
            if let Some(tx) = sender.lock().ok().and_then(|mut slot| slot.take()) {
                let _ = tx.send(result);
            }
        });

        // SAFETY: the completion handler is retained by WebKit until it runs.
        unsafe {
            view.takeSnapshotWithConfiguration_completionHandler(Some(&configuration), &handler);
        }
    })?;

    match tokio::time::timeout(std::time::Duration::from_secs(10), rx).await {
        Ok(Ok(result)) => result.map_err(anyhow::Error::msg),
        Ok(Err(_)) => Err(anyhow::anyhow!("snapshot callback was dropped")),
        Err(_) => Err(anyhow::anyhow!("snapshot timed out")),
    }
}

#[cfg(not(target_os = "macos"))]
pub async fn capture(_webview: &tauri::Webview<tauri::Wry>) -> anyhow::Result<Vec<u8>> {
    anyhow::bail!(
        "native screenshots are not implemented on this platform yet; the macOS build uses WKWebView.takeSnapshot"
    )
}
