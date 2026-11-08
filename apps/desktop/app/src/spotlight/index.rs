//! Core Spotlight write path.
//!
//! The safe Rust bindings are used directly (`objc2-core-spotlight`) so the
//! macOS build does not need Swift at compile time. All functions here run on a
//! blocking worker thread: the completion handlers are bridged back with a
//! channel, and the caller (the index queue) never runs them on the WebView
//! thread.

use super::payload::IndexItem;

/// The narrow surface the queue needs. Tests can substitute an in-memory sink;
/// non-macOS builds use [`NoopSink`].
pub trait IndexSink: Send + Sync {
    /// Why this host cannot index, if it cannot. `Ok(())` means the OS index is
    /// reachable from the current process.
    fn available(&self) -> Result<(), String>;
    fn index(&self, items: &[IndexItem]) -> anyhow::Result<()>;
    fn delete(&self, item_ids: &[String]) -> anyhow::Result<()>;
    fn delete_all(&self) -> anyhow::Result<()>;
}

#[cfg(not(target_os = "macos"))]
pub struct NoopSink;

#[cfg(not(target_os = "macos"))]
impl IndexSink for NoopSink {
    fn available(&self) -> Result<(), String> {
        Err("Spotlight indexing is only available on macOS".to_string())
    }

    fn index(&self, _items: &[IndexItem]) -> anyhow::Result<()> {
        Err(anyhow::anyhow!(
            "Spotlight indexing is only available on macOS"
        ))
    }

    fn delete(&self, _item_ids: &[String]) -> anyhow::Result<()> {
        Err(anyhow::anyhow!(
            "Spotlight indexing is only available on macOS"
        ))
    }

    fn delete_all(&self) -> anyhow::Result<()> {
        Err(anyhow::anyhow!(
            "Spotlight indexing is only available on macOS"
        ))
    }
}

pub fn default_sink() -> std::sync::Arc<dyn IndexSink> {
    #[cfg(target_os = "macos")]
    {
        std::sync::Arc::new(macos::CoreSpotlightSink)
    }
    #[cfg(not(target_os = "macos"))]
    {
        std::sync::Arc::new(NoopSink)
    }
}

#[cfg(target_os = "macos")]
mod macos {
    use std::time::Duration;

    use block2::RcBlock;
    use objc2::rc::Retained;
    use objc2::AnyThread;
    use objc2_core_spotlight::{CSSearchableIndex, CSSearchableItem, CSSearchableItemAttributeSet};
    use objc2_foundation::{NSArray, NSBundle, NSDate, NSError, NSString};
    use objc2_uniform_type_identifiers::UTTypeText;

    use super::{IndexItem, IndexSink};

    const COMPLETION_TIMEOUT: Duration = Duration::from_secs(60);

    pub struct CoreSpotlightSink;

    impl IndexSink for CoreSpotlightSink {
        fn available(&self) -> Result<(), String> {
            // A bare `target/debug/...` executable has no bundle identifier and
            // Core Spotlight rejects every write. `tauri dev` runs in exactly
            // that shape, so surface it as a normal unsupported state instead
            // of retrying an operation that cannot succeed.
            if NSBundle::mainBundle().bundleIdentifier().is_none() {
                return Err(
                    "Spotlight indexing needs a bundled app; use the installed build \
                     (`tauri build`) instead of `tauri dev`"
                        .to_string(),
                );
            }
            // CoreSpotlight is available on macOS 10.13+, but the index may be
            // unavailable in restricted sessions (for example a headless CI
            // runner). Treat that as unsupported rather than a hard error.
            if !unsafe { CSSearchableIndex::isIndexingAvailable() } {
                return Err("Core Spotlight is not available in this session".to_string());
            }
            Ok(())
        }

        fn index(&self, items: &[IndexItem]) -> anyhow::Result<()> {
            if items.is_empty() {
                return Ok(());
            }
            objc2::rc::autoreleasepool(|_| {
                let index = unsafe { CSSearchableIndex::defaultSearchableIndex() };
                let ns_items: Vec<Retained<CSSearchableItem>> =
                    items.iter().map(make_item).collect();
                let array = NSArray::from_retained_slice(&ns_items);
                let (tx, rx) = std::sync::mpsc::channel();
                let block = RcBlock::new(move |error: *mut NSError| {
                    let result = if error.is_null() {
                        Ok(())
                    } else {
                        Err(error_message(error))
                    };
                    let _ = tx.send(result);
                });
                unsafe {
                    index.indexSearchableItems_completionHandler(&array, Some(&*block));
                }
                wait(rx)
            })
        }

        fn delete(&self, item_ids: &[String]) -> anyhow::Result<()> {
            if item_ids.is_empty() {
                return Ok(());
            }
            objc2::rc::autoreleasepool(|_| {
                let index = unsafe { CSSearchableIndex::defaultSearchableIndex() };
                let ids: Vec<Retained<NSString>> =
                    item_ids.iter().map(|id| NSString::from_str(id)).collect();
                let array = NSArray::from_retained_slice(&ids);
                let (tx, rx) = std::sync::mpsc::channel();
                let block = RcBlock::new(move |error: *mut NSError| {
                    let result = if error.is_null() {
                        Ok(())
                    } else {
                        Err(error_message(error))
                    };
                    let _ = tx.send(result);
                });
                unsafe {
                    index.deleteSearchableItemsWithIdentifiers_completionHandler(
                        &array,
                        Some(&*block),
                    );
                }
                wait(rx)
            })
        }

        fn delete_all(&self) -> anyhow::Result<()> {
            objc2::rc::autoreleasepool(|_| {
                let index = unsafe { CSSearchableIndex::defaultSearchableIndex() };
                let (tx, rx) = std::sync::mpsc::channel();
                let block = RcBlock::new(move |error: *mut NSError| {
                    let result = if error.is_null() {
                        Ok(())
                    } else {
                        Err(error_message(error))
                    };
                    let _ = tx.send(result);
                });
                unsafe {
                    index.deleteAllSearchableItemsWithCompletionHandler(Some(&*block));
                }
                wait(rx)
            })
        }
    }

    fn make_item(item: &IndexItem) -> Retained<CSSearchableItem> {
        // `UTTypeText` (public.text) is the content type that Spotlight renders
        // as a normal document result. `public.content` is too generic and was
        // observed not to surface in the macOS UI.
        let attributes = unsafe {
            CSSearchableItemAttributeSet::initWithContentType(
                CSSearchableItemAttributeSet::alloc(),
                UTTypeText,
            )
        };

        let title = NSString::from_str(&item.title);
        let summary = NSString::from_str(&item.summary);
        unsafe {
            attributes.setTitle(Some(&title));
            attributes.setContentDescription(Some(&summary));
        }

        if !item.keywords.is_empty() {
            let keywords: Vec<Retained<NSString>> = item
                .keywords
                .iter()
                .map(|keyword| NSString::from_str(keyword))
                .collect();
            let array = NSArray::from_retained_slice(&keywords);
            unsafe {
                attributes.setKeywords(Some(&array));
            }
        }

        if let Some(body) = &item.body {
            let body = NSString::from_str(body);
            unsafe {
                attributes.setTextContent(Some(&body));
            }
        }

        // Dates drive Spotlight's recency ordering. `NSDate` is seconds since
        // the Unix epoch, which is exactly what chrono's timestamp returns.
        for (date, setter) in [
            (item.created_at, "creation"),
            (item.updated_at, "modification"),
        ] {
            let Some(date) = date else { continue };
            let timestamp =
                date.timestamp() as f64 + f64::from(date.timestamp_subsec_nanos()) / 1e9;
            let date = NSDate::dateWithTimeIntervalSince1970(timestamp);
            match setter {
                "creation" => unsafe { attributes.setContentCreationDate(Some(&date)) },
                _ => unsafe { attributes.setContentModificationDate(Some(&date)) },
            }
        }

        let identifier = NSString::from_str(&item.item_id);
        let domain = NSString::from_str(item.domain);
        unsafe {
            CSSearchableItem::initWithUniqueIdentifier_domainIdentifier_attributeSet(
                CSSearchableItem::alloc(),
                Some(&identifier),
                Some(&domain),
                &attributes,
            )
        }
    }

    fn wait(rx: std::sync::mpsc::Receiver<Result<(), String>>) -> anyhow::Result<()> {
        match rx.recv_timeout(COMPLETION_TIMEOUT) {
            Ok(Ok(())) => Ok(()),
            Ok(Err(message)) => Err(anyhow::anyhow!(message)),
            Err(_) => Err(anyhow::anyhow!(
                "Core Spotlight did not answer within {}s",
                COMPLETION_TIMEOUT.as_secs()
            )),
        }
    }

    fn error_message(error: *mut NSError) -> String {
        let error = unsafe { &*error };
        format!(
            "Core Spotlight failed: {} (code {})",
            error.localizedDescription(),
            error.code()
        )
    }
}
